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
    /// The persisted vectors do not match the configured identity. Startup
    /// reports this and schedules nothing: only an explicit rebuild (or a
    /// confirmed same-space acknowledgment) may re-embed or clear it.
    pub startup_requires_rebuild: bool,
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
    // A persisted identity mismatch is reported, never repaired implicitly.
    let startup_requires_rebuild = sync.runtime_configured() && vector.requires_rebuild;
    // One readiness truth for the process: a configured runtime with no
    // published pair, or with persisted vectors of a different identity, must
    // not report ready, and that state keeps the embedding handle, query
    // service, and dependency gates stopped together.
    let has_pair = vector.vector.snapshot().is_some();
    vector.gate.set_ready(has_pair && !startup_requires_rebuild);
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
        startup_requires_rebuild,
        secrets: store,
    })
}
