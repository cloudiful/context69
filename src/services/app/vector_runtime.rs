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
/// Only a persisted row proves this installation already holds vectors for an
/// identity. A missing row is the safe zero-data bootstrap path, even though
/// the configured fingerprint trivially differs from "no row", and a matching
/// legacy identity stays ready with no upgrade-wide rebuild.
pub(crate) fn requires_rebuild_from_persisted(
    runtime_configured: bool,
    stored_fingerprint: Option<&str>,
    configured_fingerprint: &str,
) -> bool {
    runtime_configured && stored_fingerprint.is_some_and(|stored| stored != configured_fingerprint)
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
    let requires_rebuild = requires_rebuild_from_persisted(
        runtime_configured,
        stored_fingerprint.as_deref(),
        &fingerprint,
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
        if self.vector.snapshot().is_none() || !self.runtime.is_ready() {
            return Err(DomainError::invalid_argument(
                "embedding identity cannot change until the vector runtime is ready with a \
                 published vector identity",
            )
            .into());
        }
        if candidate.dimensions != current.dimensions {
            return Err(DomainError::invalid_argument(
                "runtime.embedding.dimensions cannot change against existing vectors; a different \
                 vector space requires an explicit dimension migration",
            )
            .into());
        }
        if !embedding.confirm_vector_space_unchanged {
            return Err(DomainError::invalid_argument(
                "runtime.embedding model/base URL cannot change without an explicit confirmation \
                 that the new gateway/alias produces the same vector space",
            )
            .into());
        }
        // A confirmed same-space transition must not report success while a live
        // reload, rebuild, or another transition is in progress: its identity
        // would remain unapplied. Credential and timeout saves stay live-safe and
        // are not blocked, so a broken credential can still be repaired during a
        // rebuild.
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
        if !self
            .vector
            .rekey_same_space(provider.clone(), identity.clone())
        {
            self.fail_reconciliation();
            return Err(DomainError::unavailable(
                "the vector runtime could not apply the acknowledged identity; readiness was \
                 withdrawn until reconciliation",
            )
            .into());
        }
        self.runtime.install(provider, identity);
        let mut effective = self.base_config.clone();
        super::runtime_settings::apply_runtime_settings(&mut effective, &stored);
        effective.embedding.api_key = api_key;
        // The acknowledged identity is now the persisted one, so the mismatch
        // that required a rebuild no longer holds, and the reconnect source must
        // record the same fingerprint for a later manual rebuild.
        self.vector.set_requires_rebuild(false);
        self.vector
            .set_configured_fingerprint(Some(super::vector_identity::fingerprint(&effective)));
        self.recovery.adopt_acknowledged_identity(effective);
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
