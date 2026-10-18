use anyhow::Result;
use context69_translation::{TranslationDependencies, TranslationService};
use std::{sync::Arc, time::Duration};
use tracing::warn;

use crate::{
    chunking::ChunkingConfig,
    config::Config,
    db::Database,
    library_store::LibraryStore,
    services::{
        auth::AuthService,
        query::{QueryService, SharedQueryDeps},
        secret_store::{self, SecretStore},
        sync::SyncService,
        translation::TranslationPublisherAdapter,
    },
};

use super::{readiness, vector_identity, vector_runtime::VectorRuntime};

pub struct ServicesInit {
    pub translation: TranslationService,
    pub sync: SyncService,
    pub query: QueryService,
    pub automatic_rebuild_needed: bool,
    /// The shared secret store the application already built, reused by the
    /// live runtime-settings reload so it resolves keys the same way.
    pub secrets: SecretStore,
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
    let store = secret_store::build(db, config)?;
    let chunking = ChunkingConfig {
        max_chars: config.chunking.max_chars,
        overlap_chars: config.chunking.overlap_chars,
    };
    let translation = TranslationService::new(TranslationDependencies {
        pool: db.pool().clone(),
        http_client: reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()?,
        publisher: Arc::new(TranslationPublisherAdapter::with_vector_runtime(
            vector.vector.clone(),
            chunking.clone(),
        )),
        concurrency: config.scheduler.max_concurrency,
        readiness: Arc::new(readiness::LibraryEmbeddingVectorReadiness {
            store: LibraryStore::new(db.clone()),
            configuration_fingerprint: vector_identity::configuration_fingerprint(config),
        }),
    })
    .with_secret_store(store.clone());
    let sync = SyncService::with_vector_runtime(
        db.clone(),
        store.clone(),
        vector.vector.clone(),
        chunking,
        config.scheduler.max_concurrency,
        translation.clone(),
    );
    sync.reload_sources().await?;
    if let Err(error) = sync.validate_sources().await {
        warn!(error = %error, "source validation failed during startup; continuing without blocking service startup");
    }
    let automatic_rebuild_needed = sync.runtime_configured()
        && (vector.collection_needs_rebuild || vector.fingerprint_changed);
    // One readiness truth for the process: a configured runtime with no
    // published pair must not report ready, and a startup rebuild keeps the
    // embedding handle, query service, and dependency gates stopped together.
    let has_pair = vector.vector.snapshot().is_some();
    vector.gate.set_ready(has_pair && !automatic_rebuild_needed);
    let query = QueryService::with_vector_runtime(SharedQueryDeps {
        db: db.clone(),
        vector: vector.vector.clone(),
        valkey_url: config.scheduler.valkey_url.as_deref(),
        embedding_model: vector_identity::fingerprint(config),
        auth: auth.clone(),
        store: store.clone(),
    })
    .await?;

    Ok(ServicesInit {
        translation,
        sync,
        query,
        automatic_rebuild_needed,
        secrets: store,
    })
}
