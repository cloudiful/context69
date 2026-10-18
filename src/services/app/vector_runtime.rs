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
    services::secret_store::SecretStore,
    vector_runtime::VectorRuntime as SharedVectorRuntime,
};

use super::{runtime_settings, vector_rebuild::VectorRebuildTicket};

/// The process-wide vector-index gate: the readiness mirror that the embedding
/// handle, the query service, and the dependency gates observe.
///
/// The gate never owns readiness; it delegates every change to the shared
/// runtime so those observers can only see a committed state.
#[derive(Clone)]
pub struct VectorIndexGate {
    runtime: SharedVectorRuntime,
}

impl Default for VectorIndexGate {
    fn default() -> Self {
        Self::new()
    }
}

impl VectorIndexGate {
    pub fn new() -> Self {
        Self {
            runtime: SharedVectorRuntime::unconfigured(),
        }
    }

    pub fn from_runtime(runtime: &SharedVectorRuntime) -> Self {
        Self {
            runtime: runtime.clone(),
        }
    }

    /// The shared readiness mirror for the embedding handle and query service.
    pub fn flag(&self) -> Arc<AtomicBool> {
        self.runtime.readiness_flag()
    }

    pub fn set_ready(&self, ready: bool) {
        self.runtime.commit_readiness(ready);
    }

    /// The completion hook for an explicit startup/manual rebuild. The rebuild
    /// fence itself is opened by `VectorRuntime::begin_rebuild` before this
    /// ticket exists, so a settings save can never close a gate on its own.
    pub fn rebuild_ticket(&self) -> VectorRebuildTicket {
        VectorRebuildTicket::new()
    }
}

pub struct VectorRuntime {
    pub embedding: EmbeddingRuntime,
    /// The shared handle every vector consumer reads its coherent pair from.
    pub vector: SharedVectorRuntime,
    pub embedding_vector_configured: bool,
    pub collection_needs_rebuild: bool,
    pub fingerprint: String,
    pub fingerprint_changed: bool,
    /// Shared with the embedding handle and the query service, so a rebuild
    /// gates writes and reads with one flag.
    pub gate: VectorIndexGate,
    /// The one recovery controller for this process. It exists even when the
    /// instance starts unconfigured and is activated by the first committed
    /// settings save.
    pub recovery: Arc<super::vector_recovery::VectorRecoveryWiring>,
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

    // The shared handle exists for the whole process lifetime, so every consumer
    // observes the same readiness truth before and after configuration.
    let shared = SharedVectorRuntime::unconfigured();
    let gate = VectorIndexGate::from_runtime(&shared);
    let mut embedding = EmbeddingRuntime::with_readiness(None, None, gate.flag());
    let mut embedding_vector_configured = false;

    if runtime_configured {
        match build_provider(&config.embedding) {
            Ok(provider) => {
                embedding_vector_configured = true;
                shared.mark_configured(true);
                embedding = EmbeddingRuntime::with_readiness(
                    Some(provider),
                    Some(EmbeddingIdentity::from_config(&config.embedding)),
                    gate.flag(),
                );
            }
            Err(error) => {
                warn!(%error, "embedding runtime is unavailable; continuing in degraded mode");
            }
        }
    }

    let recovery =
        super::vector_recovery::startup_recovery(db, config, shared.clone(), runtime_configured)
            .await;

    // A collection that exists but has diverged needs the explicit rebuild
    // path, which is the only path allowed to recreate destructively.
    let collection_needs_rebuild = shared.snapshot().is_some()
        && fingerprint_changed
        && config.qdrant.recreate_on_dimension_mismatch;

    Ok(VectorRuntime {
        embedding,
        vector: shared,
        embedding_vector_configured,
        collection_needs_rebuild,
        fingerprint,
        fingerprint_changed,
        gate,
        recovery,
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
    vector: SharedVectorRuntime,
    reload_in_progress: Arc<AtomicBool>,
}

impl EmbeddingSettingsGuard {
    pub fn new(
        runtime: EmbeddingRuntime,
        vector: SharedVectorRuntime,
        reload_in_progress: Arc<AtomicBool>,
    ) -> Self {
        Self {
            runtime,
            vector,
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
        if identity_change && self.vector.snapshot().is_some() {
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
    pub vector: SharedVectorRuntime,
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
        if self.context.vector.snapshot().is_some()
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
        // A live pair keeps its provider identity: only a credential or timeout
        // change may replace it. With no published pair there is nothing to
        // preserve, so the first configuration activates the standalone handle
        // for the consumers that read it before the pair is published.
        if self.context.vector.snapshot().is_some()
            && !self
                .context
                .vector
                .replace_provider(provider.clone(), candidate_identity.clone())
        {
            warn!(
                "embedding identity changed under a concurrent update; keeping the previous runtime"
            );
            return Ok(());
        }
        self.context.runtime.install(provider, candidate_identity);
        Ok(())
    }
}

#[cfg(test)]
#[path = "vector_runtime_tests.rs"]
mod tests;
