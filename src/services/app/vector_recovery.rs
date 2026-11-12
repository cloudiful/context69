//! Application wiring for the vector recovery engine.
//!
//! The engine itself (`vector_runtime::recovery`) is deterministic and knows
//! nothing about the database, runtime settings, or dependency gates. This
//! module supplies:
//!
//! - the production connector: resolve the configured provider/identity, attach
//!   read-only to the active physical collection, and probe with read-only
//!   RPCs;
//! - the observer: install the recovered provider on the standalone embedding
//!   handle and report the probe outcome through the existing dependency-gate
//!   lease protocol;
//! - the bounded startup attempt and the cancellation guard that stops the
//!   ongoing supervisor when the application is dropped.

use std::sync::{Arc, RwLock};

use anyhow::Result;
use async_trait::async_trait;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::{
    config::Config,
    db::Database,
    embedding::{EmbeddingIdentity, build_provider},
    qdrant_index::QdrantIndex,
    services::library::LibraryService,
    services::settings::ConfigRefreshOutcome,
    vector_runtime::{
        VectorFailureClass, VectorRuntime, VectorSnapshot,
        recovery::{RecoveryObserver, TokioSleeper, VectorConnector, VectorPair, VectorRecovery},
    },
};

use super::{vector_identity, vector_runtime::VectorIndexGate};

/// Cancels the ongoing supervisor when the last application handle is dropped.
pub struct RecoveryShutdown(CancellationToken);

impl RecoveryShutdown {
    fn new() -> Self {
        Self(CancellationToken::new())
    }

    pub fn token(&self) -> CancellationToken {
        self.0.clone()
    }
}

impl Drop for RecoveryShutdown {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

/// The recovery collaborators the application keeps for its lifetime.
pub struct VectorRecoveryWiring {
    runtime: VectorRuntime,
    connector: Arc<AppVectorConnector>,
    shutdown: Arc<RecoveryShutdown>,
}

impl VectorRecoveryWiring {
    pub fn shutdown_token(&self) -> CancellationToken {
        self.shutdown.token()
    }

    /// Applies the authoritative committed configuration to the reconnect
    /// source while the caller holds the settings save lock and the shared
    /// runtime transition reservation.
    ///
    /// The reservation is what makes this atomic: no rebuild can start and no
    /// other transition can interleave, so the connector swap, the fingerprint,
    /// and the generation bump commit as one transition. A vector-relevant
    /// change advances the generation and withdraws readiness; a credential-only
    /// change reconnects with the same collection identity. A candidate whose
    /// provider cannot be built never replaces the current configuration.
    pub(crate) fn apply_reserved_reconfiguration(&self, config: Config) -> ConfigRefreshOutcome {
        if let Err(error) = build_provider(&config.embedding) {
            warn!(
                %error,
                "invalid embedding configuration was not applied to the vector runtime"
            );
            return ConfigRefreshOutcome::Rejected;
        }
        let previous = self.connector.configuration_fingerprint();
        let fingerprint = vector_identity::fingerprint(&config);
        let connector = self.connector.clone();
        let Some((changed, _generation)) =
            self.runtime
                .apply_runtime_reconfiguration(Some(fingerprint), move || {
                    connector.set_config(config);
                    previous != connector.configuration_fingerprint()
                })
        else {
            // A rebuild owns the runtime under the state lock, so nothing was
            // applied; the caller defers and retries from the committed row.
            return ConfigRefreshOutcome::Deferred;
        };
        self.runtime.mark_configured(true);
        if changed {
            ConfigRefreshOutcome::Applied
        } else {
            ConfigRefreshOutcome::AppliedUnchanged
        }
    }

    /// Adopts the desired configuration of a confirmed same-space identity
    /// transition without starting a new recovery generation.
    ///
    /// The published pair is rekeyed in one write: the acknowledged provider,
    /// its identity and the fingerprint of the vector space it produces move
    /// together, so the published snapshot never carries a provider of one
    /// configuration with the identity of another. Only the reconnect source
    /// for later publications changes.
    pub fn adopt_acknowledged_identity(
        &self,
        config: Config,
        provider: std::sync::Arc<dyn crate::embedding::EmbeddingProvider>,
        identity: crate::embedding::EmbeddingIdentity,
    ) -> bool {
        let fingerprint = vector_identity::fingerprint(&config);
        if !self
            .runtime
            .rekey_same_space(provider, identity, Some(fingerprint))
        {
            return false;
        }
        self.connector.set_config(config);
        self.runtime.mark_configured(true);
        true
    }

    /// Runs the bounded startup attempt once, when the instance already had
    /// committed runtime settings at boot.
    pub async fn run_startup_attempt(self: &Arc<Self>) {
        let engine = VectorRecovery::new(
            self.runtime.clone(),
            self.connector.clone(),
            Arc::new(TokioSleeper),
        );
        let connected = engine.run_startup(&self.shutdown.token()).await;
        if !connected && self.runtime.snapshot().is_none() {
            warn!(
                "qdrant runtime is unavailable after the startup retry budget; continuing with management reachable and vector work unavailable"
            );
        }
    }

    /// Spawns the ongoing supervisor with the application's observer.
    pub fn spawn(self: &Arc<Self>, observer: Arc<dyn RecoveryObserver>) {
        drop(self.spawn_owned(observer));
    }

    /// Spawns the ongoing supervisor and returns its join handle, so a caller
    /// that must prove the supervisor stopped can await it.
    pub fn spawn_owned(
        self: &Arc<Self>,
        observer: Arc<dyn RecoveryObserver>,
    ) -> tokio::task::JoinHandle<()> {
        let engine = VectorRecovery::new(
            self.runtime.clone(),
            self.connector.clone(),
            Arc::new(TokioSleeper),
        )
        .with_observer(observer);
        let shutdown = self.shutdown.token();
        tokio::spawn(async move {
            engine.supervise(shutdown).await;
        })
    }
}

struct AppVectorConnector {
    db: Database,
    config: RwLock<Config>,
}

impl AppVectorConnector {
    fn config(&self) -> Config {
        self.config
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn set_config(&self, config: Config) {
        *self
            .config
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = config;
    }
}

#[async_trait]
impl VectorConnector for AppVectorConnector {
    async fn pair(&self) -> Result<VectorPair> {
        let config = self.config();
        let embedding = build_provider(&config.embedding)?;
        let identity = EmbeddingIdentity::from_config(&config.embedding);
        let configured_fingerprint = vector_identity::fingerprint(&config);
        // A durably invalid derived index (a failed, cancelled or superseded
        // installation) must not be advertised as a healthy cached vector pair
        // after a restart. Existing rebuild-required semantics apply.
        if self.db.read_search_registry_unavailable().await? {
            return Err(anyhow::anyhow!(
                "configuration: the derived original index is invalid; an explicit rebuild must \
                 install and activate a matching collection"
            ));
        }
        // The active pointer is the only collection this deployment may serve.
        // Without it there are no vectors to read at all, and an active row
        // whose logical identity or dimension differs from the configured one
        // describes a different vector space. Both fail closed: startup never
        // creates, adopts, or empties a collection.
        let active = self
            .db
            .read_active_vector_index_state()
            .await?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "configuration: no active vector index is recorded; an explicit rebuild must \
                     activate a vector collection before the runtime can serve vector traffic"
                )
            })?;
        if active.fingerprint != configured_fingerprint
            || active.dimensions != config.embedding.dimensions
        {
            return Err(anyhow::anyhow!(
                "configuration: the active vector index does not match the configured vector \
                 identity; an explicit rebuild must activate a matching collection"
            ));
        }
        Ok(VectorPair {
            embedding,
            identity,
            collection: active.collection_name,
            configured_fingerprint: Some(configured_fingerprint),
        })
    }

    async fn attach(&self, pair: &VectorPair) -> Result<Arc<QdrantIndex>> {
        let config = self.config();
        // The active collection is only ever read: the attach performs the
        // existence, dimension, and count checks and nothing else.
        let index = QdrantIndex::attach_read_only_collection(
            &config.qdrant,
            &pair.collection,
            config.embedding.dimensions,
        )
        .await?;
        Ok(Arc::new(index))
    }

    async fn probe(&self, snapshot: &VectorSnapshot) -> Result<()> {
        snapshot.index.probe_collection().await
    }

    fn configuration_fingerprint(&self) -> String {
        vector_identity::configuration_fingerprint(&self.config())
    }
}

pub struct AppRecoveryObserver {
    embedding: crate::embedding::EmbeddingRuntime,
    library: LibraryService,
    document_store: crate::services::document_store::DocumentStoreService,
    /// The normal profile resumes the metadata-index worker when a pair is
    /// published. The upgrade-recovery profile does not own that writer and
    /// must not start it before the migration.
    resume_document_store: bool,
}

impl AppRecoveryObserver {
    pub fn new(
        embedding: crate::embedding::EmbeddingRuntime,
        library: LibraryService,
        document_store: crate::services::document_store::DocumentStoreService,
    ) -> Self {
        Self::with_document_store_resume(embedding, library, document_store, true)
    }

    /// Recovery-profile observer: refreshes dependency gates but never starts
    /// the metadata-index worker.
    pub fn without_document_store_resume(
        embedding: crate::embedding::EmbeddingRuntime,
        library: LibraryService,
        document_store: crate::services::document_store::DocumentStoreService,
    ) -> Self {
        Self::with_document_store_resume(embedding, library, document_store, false)
    }

    fn with_document_store_resume(
        embedding: crate::embedding::EmbeddingRuntime,
        library: LibraryService,
        document_store: crate::services::document_store::DocumentStoreService,
        resume_document_store: bool,
    ) -> Self {
        Self {
            embedding,
            library,
            document_store,
            resume_document_store,
        }
    }
}

#[async_trait]
impl RecoveryObserver for AppRecoveryObserver {
    async fn published(&self, snapshot: &VectorSnapshot) {
        if let Some(identity) = snapshot.identity.clone() {
            self.embedding.install(snapshot.embedding.clone(), identity);
        }
        if let Err(error) = self.library.refresh_dependency_configuration().await {
            warn!(%error, "failed to refresh dependency gates after vector runtime recovery");
        }
        // A metadata index queued while Qdrant was unavailable skipped its
        // remote field-index step. Resume the backlog now that a safe pair is
        // published instead of leaving it permanently incomplete.
        if self.resume_document_store {
            self.document_store.resume_pending();
        }
    }

    async fn failed(&self, generation: u64, class: VectorFailureClass, message: &str) {
        warn!(generation, class = ?class, error = %message, "vector runtime recovery failed");
        self.library
            .note_vector_runtime_failure(class == VectorFailureClass::Configuration, message)
            .await;
    }

    async fn probe_failed(&self, snapshot: &VectorSnapshot, message: &str) {
        warn!(
            generation = snapshot.generation,
            error = %message,
            "vector runtime probe failed"
        );
        self.library
            .note_vector_runtime_failure(false, message)
            .await;
    }

    async fn probe_recovered(&self, _snapshot: &VectorSnapshot) {
        self.library.note_vector_runtime_recovered().await;
    }
}

/// Builds the one recovery controller this process owns.
///
/// The controller exists even when the instance starts without runtime
/// settings: it holds the shared handle, the reconnect source, and the
/// cancellation token, so a later valid settings save activates the supervisor
/// and the outbox without a restart. Nothing is probed until configuration is
/// committed.
pub async fn startup_recovery(
    db: &Database,
    config: &Config,
    runtime: VectorRuntime,
    runtime_configured: bool,
) -> Arc<VectorRecoveryWiring> {
    let gate = VectorIndexGate::from_runtime(&runtime);
    gate.set_ready(false);
    let connector = Arc::new(AppVectorConnector {
        db: db.clone(),
        config: RwLock::new(config.clone()),
    });
    let wiring = Arc::new(VectorRecoveryWiring {
        runtime: runtime.clone(),
        connector,
        shutdown: Arc::new(RecoveryShutdown::new()),
    });
    if runtime_configured && runtime.is_configured() {
        wiring.run_startup_attempt().await;
    }
    wiring
}
