use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use anyhow::Result;
use tokio::sync::Mutex;
use tracing::warn;

use crate::{
    config::Config,
    db::Database,
    domain_errors::DomainError,
    embedding::{EmbeddingIdentity, EmbeddingRuntime, build_provider},
    qdrant_index::QdrantIndex,
    services::secret_store::SecretStore,
};

use super::{runtime_settings, vector_rebuild::VectorRebuildTicket};

/// The process-wide vector-index gate: the shared readiness flag that the
/// embedding handle, the query service, the rebuild tickets and the dependency
/// gates all observe.
#[derive(Clone)]
pub struct VectorIndexGate {
    ready: Arc<AtomicBool>,
}

impl Default for VectorIndexGate {
    fn default() -> Self {
        Self::new()
    }
}

impl VectorIndexGate {
    pub fn new() -> Self {
        Self {
            ready: Arc::new(AtomicBool::new(true)),
        }
    }

    /// The shared readiness flag for the embedding handle and query service.
    pub fn flag(&self) -> Arc<AtomicBool> {
        self.ready.clone()
    }

    pub fn set_ready(&self, ready: bool) {
        self.ready.store(ready, Ordering::Release);
    }

    /// The completion ticket for an explicit startup/manual rebuild. It marks
    /// the index unready until the rebuild succeeds. A settings save never
    /// creates one.
    pub fn rebuild_ticket(&self) -> VectorRebuildTicket {
        VectorRebuildTicket::new(self.ready.clone())
    }
}

pub struct VectorRuntime {
    pub embedding: EmbeddingRuntime,
    pub index: Option<QdrantIndex>,
    pub embedding_vector_configured: bool,
    pub collection_needs_rebuild: bool,
    pub fingerprint: String,
    pub fingerprint_changed: bool,
    /// Shared with the embedding handle and the query service, so a rebuild
    /// gates writes and reads with one flag.
    pub gate: VectorIndexGate,
}

pub async fn initialize(
    db: &Database,
    config: &Config,
    runtime_configured: bool,
) -> Result<VectorRuntime> {
    let fingerprint = super::vector_identity::fingerprint(config);
    let stored_fingerprint = db
        .get_vector_index_fingerprint(&config.qdrant.collection_name)
        .await?;
    let fingerprint_changed = stored_fingerprint.as_deref() != Some(&fingerprint);

    // Created before the provider so the embedding handle and the query path
    // share one readiness gate for the whole process.
    let gate = VectorIndexGate::new();
    let mut embedding = EmbeddingRuntime::with_readiness(None, None, gate.flag());
    let mut index: Option<QdrantIndex> = None;
    let mut embedding_vector_configured = false;
    let mut collection_needs_rebuild = false;

    if runtime_configured {
        match build_provider(&config.embedding) {
            Ok(provider) => {
                embedding_vector_configured = true;
                let identity = EmbeddingIdentity::from_config(&config.embedding);
                let mut qdrant_config = config.qdrant.clone();
                qdrant_config.recreate_on_dimension_mismatch |= fingerprint_changed;
                match QdrantIndex::connect(&qdrant_config, config.embedding.dimensions).await {
                    Ok((connected_index, recreated)) => {
                        embedding = EmbeddingRuntime::with_readiness(
                            Some(provider),
                            Some(identity),
                            gate.flag(),
                        );
                        index = Some(connected_index);
                        collection_needs_rebuild = recreated;
                    }
                    Err(error) => {
                        warn!(error = %error, "qdrant runtime is unavailable; continuing in degraded mode");
                    }
                }
            }
            Err(error) => {
                warn!(%error, "embedding runtime is unavailable; continuing in degraded mode");
            }
        }
    }

    Ok(VectorRuntime {
        embedding,
        index,
        embedding_vector_configured,
        collection_needs_rebuild,
        fingerprint,
        fingerprint_changed,
        gate,
    })
}

/// Rejects a runtime embedding update that the live runtime cannot apply.
///
/// While a fixed vector index is live, only credential and timeout changes are
/// hot-swappable: the API key and timeout are not part of the identity, so a
/// change to either installs live. A model, dimension, or base-URL change would
/// need a full reindex and is rejected before anything is persisted, so the
/// save fails instead of reporting success while the live runtime stays on the
/// old identity. An identity-changing save is likewise rejected while a live
/// reload or rebuild is in progress, and any save is rejected while a reload is
/// applying, so a racing save can never report success while unapplied.
#[derive(Clone)]
pub struct EmbeddingSettingsGuard {
    runtime: EmbeddingRuntime,
    index_present: bool,
    reload_in_progress: Arc<AtomicBool>,
}

impl EmbeddingSettingsGuard {
    pub fn new(
        runtime: EmbeddingRuntime,
        index_present: bool,
        reload_in_progress: Arc<AtomicBool>,
    ) -> Self {
        Self {
            runtime,
            index_present,
            reload_in_progress,
        }
    }

    pub fn check(
        &self,
        embedding: &crate::contracts::UpdateRuntimeEmbeddingSettings,
    ) -> Result<()> {
        let candidate = EmbeddingIdentity::from_update(embedding);
        let identity_change = self
            .runtime
            .current_identity()
            .is_some_and(|current| current != candidate);
        if identity_change && self.index_present {
            return Err(DomainError::invalid_argument(
                "runtime.embedding model/base URL/dimensions cannot change while the vector index \
                 is live; only credential and timeout changes apply without a rebuild",
            )
            .into());
        }
        // An identity-changing save must not report success while a live reload
        // or rebuild is in progress: its identity would remain unapplied.
        // Credential and timeout saves stay live-safe and are not blocked, so a
        // broken credential can still be repaired during a rebuild.
        if identity_change
            && (self.reload_in_progress.load(Ordering::Acquire) || !self.runtime.is_ready())
        {
            return Err(DomainError::invalid_argument(
                "embedding identity cannot change while a runtime reload or rebuild is in \
                 progress; retry after it completes",
            )
            .into());
        }
        Ok(())
    }
}

/// Marks a live runtime reload as in progress for the duration of its scope.
struct ReloadInProgressGuard(Arc<AtomicBool>);

impl ReloadInProgressGuard {
    fn begin(flag: Arc<AtomicBool>) -> Self {
        flag.store(true, Ordering::Release);
        Self(flag)
    }
}

impl Drop for ReloadInProgressGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// The collaborators the live embedding-runtime reloader is built from. Grouped
/// so the constructor stays a single value instead of a long positional list.
pub struct EmbeddingRuntimeReloaderContext {
    pub db: Database,
    pub store: SecretStore,
    pub runtime: EmbeddingRuntime,
    pub index_present: bool,
    pub base_config: Config,
    pub reload_in_progress: Arc<AtomicBool>,
}

/// Applies a credential/timeout-only runtime settings change to the shared
/// embedding handle.
///
/// The persisted row is already committed when this runs, so it is the source
/// of truth. The reloader never changes the identity and never starts a
/// rebuild: an identity-changing save is rejected by [`EmbeddingSettingsGuard`]
/// before it is committed, so a candidate whose identity differs from the live
/// one is left uninstalled and logged rather than mixed into the collection.
pub struct EmbeddingRuntimeReloader {
    context: EmbeddingRuntimeReloaderContext,
    lock: Arc<Mutex<()>>,
}

impl EmbeddingRuntimeReloader {
    pub fn new(context: EmbeddingRuntimeReloaderContext) -> Self {
        Self {
            context,
            lock: Arc::new(Mutex::new(())),
        }
    }

    pub async fn reload(&self) -> Result<()> {
        // Serialize concurrent applies so two saves cannot interleave their
        // installs; the lock is held across the settings read and swap only,
        // never across an embedding request.
        let _guard = self.lock.lock().await;
        let _reloading = ReloadInProgressGuard::begin(self.context.reload_in_progress.clone());

        let Some(stored) =
            runtime_settings::load_runtime_settings(&self.context.db, &self.context.store).await?
        else {
            return Ok(());
        };
        let mut effective = self.context.base_config.clone();
        runtime_settings::apply_runtime_settings(&mut effective, &stored);

        let candidate_identity = EmbeddingIdentity::from_config(&effective.embedding);
        let previous = self.context.runtime.current_identity();
        if self.context.index_present
            && previous
                .as_ref()
                .is_some_and(|previous| previous != &candidate_identity)
        {
            warn!(
                "embedding identity change was not applied live; only credential and timeout \
                 changes are hot-swappable while a fixed vector index is live"
            );
            return Ok(());
        }

        let provider = match build_provider(&effective.embedding) {
            Ok(provider) => provider,
            Err(error) => {
                warn!(
                    %error,
                    "embedding runtime replacement could not be built; keeping the previous runtime"
                );
                return Ok(());
            }
        };
        self.context.runtime.install(provider, candidate_identity);
        Ok(())
    }
}

#[cfg(test)]
#[path = "vector_runtime_tests.rs"]
mod tests;
