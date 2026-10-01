use std::{
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

use anyhow::Result;
use context69_extraction::{ExtractionDependencies, ExtractionService};
use context69_translation::{TranslationDependencies, TranslationService};
use tracing::warn;

use crate::{
    chunking::ChunkingConfig,
    config::Config,
    db::Database,
    library_store::LibraryStore,
    services::{
        auth::AuthService,
        extraction::ExtractionPublisherAdapter,
        query::{QueryDeps, QueryService},
        secret_store,
        sync::SyncService,
        translation::TranslationPublisherAdapter,
    },
};

use super::{readiness, vector_identity, vector_runtime::VectorRuntime};

pub struct ServicesInit {
    pub translation: TranslationService,
    pub extraction: ExtractionService,
    pub sync: SyncService,
    pub query: QueryService,
    pub vector_index_ready: Arc<AtomicBool>,
    pub automatic_rebuild_needed: bool,
}

pub async fn initialize(
    db: &Database,
    config: &Config,
    auth: &AuthService,
    vector: &VectorRuntime,
) -> Result<ServicesInit> {
    // The search path needs the rerank key resolved through the shared store, so
    // it gets a handle onto the same configuration the application already
    // built. Building a handle is just a cipher over the existing pool; the
    // unconfigured state was reported once during config hydration.
    let store = secret_store::build(db, &config.secret_store)?;
    let translation = TranslationService::new(TranslationDependencies {
        pool: db.pool().clone(),
        http_client: reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()?,
        publisher: Arc::new(TranslationPublisherAdapter::new(
            vector.embedding.clone(),
            vector.index.clone(),
            ChunkingConfig {
                max_chars: config.chunking.max_chars,
                overlap_chars: config.chunking.overlap_chars,
            },
        )),
        concurrency: config.scheduler.max_concurrency,
        readiness: Arc::new(readiness::LibraryEmbeddingVectorReadiness {
            store: LibraryStore::new(db.clone()),
            configuration_fingerprint: vector_identity::configuration_fingerprint(config),
        }),
    });
    let extraction = ExtractionService::new(ExtractionDependencies {
        pool: db.pool().clone(),
        http_client: reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()?,
        publisher: Arc::new(ExtractionPublisherAdapter::new(
            db.clone(),
            vector.index.clone(),
        )),
        concurrency: config.scheduler.max_concurrency,
        readiness: Arc::new(readiness::LibraryEmbeddingVectorReadiness {
            store: LibraryStore::new(db.clone()),
            configuration_fingerprint: vector_identity::configuration_fingerprint(config),
        }),
    });
    let sync = SyncService::new(
        db.clone(),
        vector.embedding.clone(),
        vector.index.clone(),
        ChunkingConfig {
            max_chars: config.chunking.max_chars,
            overlap_chars: config.chunking.overlap_chars,
        },
        config.scheduler.max_concurrency,
        translation.clone(),
    );
    sync.reload_sources().await?;
    if let Err(error) = sync.validate_sources().await {
        warn!(error = %error, "source validation failed during startup; continuing without blocking service startup");
    }
    let automatic_rebuild_needed = sync.runtime_configured()
        && (vector.collection_needs_rebuild || vector.fingerprint_changed);
    let vector_index_ready = Arc::new(AtomicBool::new(!automatic_rebuild_needed));
    if automatic_rebuild_needed {
        sync.begin_vector_index_rebuild().await?;
    }
    let query =
        if let (Some(embedding), Some(index)) = (vector.embedding.clone(), vector.index.clone()) {
            QueryService::new(QueryDeps {
                db: db.clone(),
                embedding,
                index,
                valkey_url: config.scheduler.valkey_url.as_deref(),
                embedding_model: vector_identity::fingerprint(config),
                auth: auth.clone(),
                store: store.clone(),
                vector_index_ready: vector_index_ready.clone(),
            })
            .await?
        } else {
            QueryService::disabled(db.clone())
        };

    Ok(ServicesInit {
        translation,
        extraction,
        sync,
        query,
        vector_index_ready,
        automatic_rebuild_needed,
    })
}
