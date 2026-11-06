use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use anyhow::Result;
use tokio::sync::Mutex;
use tracing::warn;

use crate::{
    config::Config,
    db::{Database, StoredRuntimeSettings},
    domain_errors::DomainError,
    embedding::{EmbeddingIdentity, EmbeddingProvider, EmbeddingRuntime, build_provider},
    vector_runtime::VectorRuntime as SharedVectorRuntime,
};

use super::runtime_settings;

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
}

pub struct VectorRuntime {
    pub embedding: EmbeddingRuntime,
    /// The shared handle every vector consumer reads its coherent pair from.
    pub vector: SharedVectorRuntime,
    pub embedding_vector_configured: bool,
    /// The persisted vectors do not match the configured identity, so readiness
    /// stays off until an explicit rebuild (or a confirmed same-space
    /// acknowledgment) succeeds. Never schedules work by itself.
    pub requires_rebuild: bool,
    /// Shared with the embedding handle and the query service, so a rebuild
    /// gates writes and reads with one flag.
    pub gate: VectorIndexGate,
    /// The one recovery controller for this process. It exists even when the
    /// instance starts unconfigured and is activated by the first committed
    /// settings save.
    pub recovery: Arc<super::vector_recovery::VectorRecoveryWiring>,
}

/// The startup truth table for a persisted identity mismatch.
///
/// The active pointer is the authority of the live vector space: only the
/// physical collection it names holds the vectors ordinary writers may use. A
/// deployment without an active collection has no vectors at all, so it owes
/// an explicit rebuild even when nothing was ever persisted, and a row whose
/// logical identity or dimension differs from the configured one describes a
/// different vector space and fails closed.
pub(crate) fn requires_rebuild_from_persisted(
    runtime_configured: bool,
    active: Option<&crate::db::ActiveVectorIndexState>,
    configured_fingerprint: &str,
    configured_dimensions: usize,
) -> bool {
    if !runtime_configured {
        // An unconfigured runtime has no vector work to gate.
        return false;
    }
    match active {
        None => true,
        Some(active) => {
            active.fingerprint != configured_fingerprint
                || active.dimensions != configured_dimensions
        }
    }
}

pub async fn initialize(
    db: &Database,
    config: &Config,
    runtime_configured: bool,
) -> Result<VectorRuntime> {
    let fingerprint = super::vector_identity::fingerprint(config);
    // The active pointer is read globally: the collection that holds the
    // vectors is the one the activation published, not the logical collection
    // name the settings carry.
    let active = db.read_active_vector_index_state().await?;
    let requires_rebuild = requires_rebuild_from_persisted(
        runtime_configured,
        active.as_ref(),
        &fingerprint,
        config.embedding.dimensions,
    );

    // The shared handle exists for the whole process lifetime, so every consumer
    // observes the same readiness truth before and after configuration.
    let shared = SharedVectorRuntime::unconfigured();
    let gate = VectorIndexGate::from_runtime(&shared);
    let mut embedding = EmbeddingRuntime::with_readiness(None, None, gate.flag());
    let mut embedding_vector_configured = false;
    shared.set_configured_fingerprint(runtime_configured.then(|| fingerprint.clone()));
    shared.set_requires_rebuild(requires_rebuild);

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

    Ok(VectorRuntime {
        embedding,
        vector: shared,
        embedding_vector_configured,
        requires_rebuild,
        gate,
        recovery,
    })
}

/// Rejects a runtime embedding update that the live runtime cannot apply.
///
/// A desired identity or dimension change is permitted while the runtime is
/// quiet: it is saved as the desired configuration, and the active collection
/// keeps serving until an explicit rebuild activates a fresh target for it.
/// Only a live transition refuses it, and it refuses it before anything is
/// persisted, so a save never reports success while the change it describes is
/// still unapplied.
#[derive(Clone)]
pub struct EmbeddingSettingsGuard {
    runtime: EmbeddingRuntime,
    vector: SharedVectorRuntime,
    reload_in_progress: Arc<AtomicBool>,
    reconciliation_required: Arc<AtomicBool>,
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
            reconciliation_required: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Shares the flag the applier sets when a committed transition could not be
    /// applied, so further identity saves are refused until reconciliation.
    pub fn with_reconciliation_required(mut self, flag: Arc<AtomicBool>) -> Self {
        self.reconciliation_required = flag;
        self
    }

    pub fn check(
        &self,
        embedding: &crate::contracts::UpdateRuntimeEmbeddingSettings,
    ) -> Result<()> {
        let candidate = EmbeddingIdentity::from_update(embedding);
        let Some(current) = self.runtime.current_identity() else {
            // First configuration: no live vector space exists yet, so there is
            // nothing to mix and the bootstrap path applies.
            return Ok(());
        };
        if current == candidate {
            // Credential and timeout are not part of the identity, so this is
            // the existing live-safe path.
            return Ok(());
        }

        if self.reconciliation_required.load(Ordering::Acquire) {
            return Err(DomainError::invalid_argument(
                "a previous vector identity transition could not be reconciled; the runtime \
                 must recover before the identity can change again",
            )
            .into());
        }
        // A desired identity change is refused while a live reload, rebuild, or
        // another transition is in progress: the identity would be saved while
        // the runtime still applies the old one. Credential and timeout saves
        // stay live-safe and are not blocked, so a broken credential can still
        // be repaired during a rebuild.
        if self.reload_in_progress.load(Ordering::Acquire)
            || self.vector.is_rebuilding()
            || self.vector.is_transitioning()
        {
            return Err(DomainError::invalid_argument(
                "embedding identity cannot change while a runtime reload, rebuild, or identity \
                 transition is in progress; retry after it completes",
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

    /// Installs one already-loaded committed configuration while the caller owns
    /// the settings save lock and the shared runtime transition.
    ///
    /// The authoritative row must be read under those locks, so this variant
    /// never reads or reserves anything itself.
    pub async fn reload_reserved(&self, stored: &StoredRuntimeSettings) -> Result<()> {
        let _guard = self.lock.lock().await;
        self.install(stored).await
    }

    async fn install(&self, stored: &StoredRuntimeSettings) -> Result<()> {
        let _reloading = ReloadInProgressGuard::begin(self.context.reload_in_progress.clone());

        let mut effective = self.context.base_config.clone();
        runtime_settings::apply_runtime_settings(&mut effective, stored);

        let candidate_identity = EmbeddingIdentity::from_config(&effective.embedding);
        // The published pair is the authority of the live vector space: the
        // collection holds the vectors of that identity, so a credential or
        // timeout change may replace its provider while an identity change
        // may not. The comparison uses the published identity rather than the
        // standalone handle, which a rebuild that activated a different
        // vector space leaves behind.
        let published = self
            .context
            .vector
            .snapshot()
            .and_then(|snapshot| snapshot.identity.clone());
        if published
            .as_ref()
            .is_some_and(|published| published != &candidate_identity)
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

/// Applies a committed, confirmed same-space identity transition to the live
/// runtime.
///
/// The provider was built before the settings transaction. The published pair is
/// rekeyed in one write-lock step, preserving its Qdrant handle and generation,
/// and only then does the standalone embedding handle and the recovery reconnect
/// source follow. A failure withdraws readiness and blocks further identity
/// saves until reconciliation instead of reporting a mixed pair as healthy.
pub struct EmbeddingTransitionApplier {
    runtime: EmbeddingRuntime,
    vector: SharedVectorRuntime,
    recovery: Arc<super::vector_recovery::VectorRecoveryWiring>,
    base_config: Config,
    reconciliation_required: Arc<AtomicBool>,
}

impl EmbeddingTransitionApplier {
    pub fn new(
        runtime: EmbeddingRuntime,
        vector: SharedVectorRuntime,
        recovery: Arc<super::vector_recovery::VectorRecoveryWiring>,
        base_config: Config,
        reconciliation_required: Arc<AtomicBool>,
    ) -> Self {
        Self {
            runtime,
            vector,
            recovery,
            base_config,
            reconciliation_required,
        }
    }

    pub async fn apply(
        &self,
        provider: Arc<dyn EmbeddingProvider>,
        identity: EmbeddingIdentity,
        stored: StoredRuntimeSettings,
        api_key: Option<String>,
    ) -> Result<()> {
        let mut effective = self.base_config.clone();
        super::runtime_settings::apply_runtime_settings(&mut effective, &stored);
        effective.embedding.api_key = api_key;
        // The published pair is rekeyed in one write: the acknowledged
        // provider, its identity and the fingerprint of the vector space they
        // produce move together, so the pair is never described by the
        // fingerprint of another configuration.
        if !self.recovery.adopt_acknowledged_identity(
            effective.clone(),
            provider.clone(),
            identity.clone(),
        ) {
            self.fail_reconciliation();
            return Err(DomainError::unavailable(
                "the vector runtime could not apply the acknowledged identity; readiness was \
                 withdrawn until reconciliation",
            )
            .into());
        }
        self.runtime.install(provider, identity);
        // The acknowledged identity is now the persisted one, so the mismatch
        // that required a rebuild no longer holds.
        self.vector.set_requires_rebuild(false);
        Ok(())
    }

    fn fail_reconciliation(&self) {
        self.vector.withdraw_readiness();
        self.reconciliation_required.store(true, Ordering::Release);
    }
}

#[cfg(test)]
#[path = "vector_runtime_tests.rs"]
mod tests;
