//! The process-wide vector runtime handle: one clone-shared, generation-fenced
//! cell holding the embedding provider and the vector index that stores its
//! output.
//!
//! Every vector consumer takes an owned snapshot of the pair, so a late-starting
//! or restored Qdrant becomes usable everywhere without restarting.
//!
//! Concurrency contract:
//!
//! - generation, rebuild fence, readiness, failure, and the published pair live
//!   in one locked state. Every transition and every readiness decision is one
//!   synchronization boundary, so a stale attempt can neither install a client
//!   nor reopen readiness for a superseded generation;
//! - the state lock is never held across network I/O, an embedding request, a
//!   sleep, or rebuild work;
//! - the readiness and rebuild atomics handed to other components only mirror a
//!   committed state. They are written under the state write lock and are never
//!   read back as this handle's source of truth.

pub mod recovery;

use std::sync::{
    Arc, RwLock,
    atomic::{AtomicBool, Ordering},
};

use anyhow::Result;

use crate::{
    domain_errors::DomainError,
    embedding::{EmbeddingIdentity, EmbeddingProvider, EmbeddingRuntime},
    qdrant_index::QdrantIndex,
};

/// How a failed vector runtime attempt should be treated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VectorFailureClass {
    /// Reachability, timeout, transport, or a not-yet-ready server: retry with
    /// backoff.
    Transient,
    /// Authentication, dimension, invalid configuration, or a missing
    /// collection that local state says must exist: retry would storm the
    /// remote and cannot heal, so the supervisor pauses until configuration
    /// changes.
    Configuration,
}

#[derive(Debug, Clone)]
pub struct VectorFailure {
    pub generation: u64,
    pub class: VectorFailureClass,
    pub message: String,
}

/// The persisted-vector identity a confirmed same-space transition replaces.
///
/// The acknowledgment is applied in the settings transaction that saves the new
/// identity, conditioned on the collection, the fingerprint currently recorded,
/// and its dimensions. Only that conditional row update may rewrite the
/// fingerprint; a missing or mismatched row fails the whole save.
#[derive(Debug, Clone, Copy)]
pub struct VectorIdentityAck<'a> {
    pub collection_name: &'a str,
    pub expected_fingerprint: &'a str,
    pub expected_dimensions: usize,
    pub candidate_fingerprint: &'a str,
    pub embedding_base_url: &'a str,
    pub embedding_model: &'a str,
}

/// One coherent pair of an embedding provider and the vector index that stores
/// its output. Cloned arcs are owned snapshots: the state lock is not held
/// while an operation runs.
#[derive(Clone)]
pub struct VectorSnapshot {
    pub embedding: Arc<dyn EmbeddingProvider>,
    pub identity: Option<EmbeddingIdentity>,
    pub index: Arc<QdrantIndex>,
    pub collection: String,
    pub generation: u64,
}

impl VectorSnapshot {
    /// A stable key for the vector space this snapshot produces, used to scope
    /// cached query vectors.
    pub fn cache_identity(&self) -> String {
        match &self.identity {
            Some(identity) => format!(
                "{}|{}|{}",
                identity.base_url, identity.model, identity.dimensions
            ),
            None => format!("provider:{:p}", Arc::as_ptr(&self.embedding) as *const ()),
        }
    }
}

/// One coherent read of the handle. Taken under a single read lock, so the
/// snapshot, its generation, the rebuild fence, and readiness always describe
/// the same committed state.
#[derive(Clone)]
pub struct CurrentVectorState {
    pub generation: u64,
    pub ready: bool,
    pub rebuilding: bool,
    pub snapshot: Option<VectorSnapshot>,
}

/// A rebuild context admitted atomically from the runtime's own committed
/// state: the generation this attempt owns, the coherent provider/index pair it
/// re-embeds with, and the configured fingerprint that pair belongs to.
pub struct AdmittedRebuild {
    pub generation: u64,
    pub snapshot: VectorSnapshot,
    pub configured_fingerprint: String,
}

/// Why an explicit rebuild could not be admitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebuildAdmissionError {
    /// A rebuild, reload, identity transition, or another rebuild start owns the
    /// shared exclusion.
    Busy,
    /// No pair is published, so there is nothing to rebuild.
    NotConfigured,
    /// The published pair has no usable identity or collection.
    MissingIdentity,
    /// The retained snapshot belongs to an older generation than the configured
    /// one; relabelling it would rebuild one collection under another's
    /// identity.
    StaleSnapshot,
    /// The configured identity fingerprint is unknown.
    MissingFingerprint,
}

impl RebuildAdmissionError {
    pub fn message(self) -> &'static str {
        match self {
            Self::Busy => {
                "vector index rebuild cannot start while the vector runtime is changing or \
                 rebuilding"
            }
            Self::NotConfigured => {
                "vector index rebuild cannot start before a vector pair is published"
            }
            Self::MissingIdentity => {
                "vector index rebuild cannot start without a configured vector identity"
            }
            Self::StaleSnapshot => {
                "vector index rebuild cannot start from a snapshot of a superseded configuration"
            }
            Self::MissingFingerprint => "configured vector identity fingerprint is unavailable",
        }
    }
}

struct VectorRuntimeState {
    generation: u64,
    rebuilding: bool,
    ready: bool,
    /// The persisted vectors do not match the configured identity, or a
    /// populated collection has no recorded identity. Readiness is never
    /// admitted while this is set; only an explicit rebuild success (or a
    /// confirmed same-space acknowledgment) clears it.
    requires_rebuild: bool,
    snapshot: Option<VectorSnapshot>,
    failure: Option<VectorFailure>,
    /// The fingerprint of the configured vector identity, recorded so a manual
    /// rebuild persists the identity it actually re-embedded with. It lives in
    /// the same state as the snapshot so admission reads both coherently.
    configured_fingerprint: Option<String>,
}

impl Default for VectorRuntimeState {
    fn default() -> Self {
        Self {
            generation: 1,
            rebuilding: false,
            ready: false,
            requires_rebuild: false,
            snapshot: None,
            failure: None,
            configured_fingerprint: None,
        }
    }
}

/// The process-wide vector runtime handle.
#[derive(Clone)]
pub struct VectorRuntime {
    state: Arc<RwLock<VectorRuntimeState>>,
    ready_mirror: Arc<AtomicBool>,
    rebuild_mirror: Arc<AtomicBool>,
    configured: Arc<AtomicBool>,
    activation: Arc<tokio::sync::Notify>,
    /// One shared exclusion for an identity transition, the authoritative
    /// runtime refresh, and a manual rebuild start. Only a settings save or the
    /// refresh holds it across the committed apply.
    identity_transition: Arc<AtomicBool>,
    /// Test-only pause point before the exclusion is reserved, used to prove the
    /// reservation-then-inspect ordering deterministically.
    #[cfg(test)]
    admission_pause: Arc<std::sync::Mutex<Option<Box<dyn FnOnce() + Send>>>>,
}

impl Default for VectorRuntime {
    fn default() -> Self {
        Self::unconfigured()
    }
}

impl VectorRuntime {
    /// A handle with no snapshot and a "not configured" classification.
    pub fn unconfigured() -> Self {
        Self::with_state(false, Arc::new(AtomicBool::new(false)))
    }

    /// A handle whose configuration is known but whose client may not be
    /// established yet.
    pub fn configured(ready_mirror: Arc<AtomicBool>) -> Self {
        Self::with_state(true, ready_mirror)
    }

    fn with_state(configured: bool, ready_mirror: Arc<AtomicBool>) -> Self {
        ready_mirror.store(false, Ordering::Release);
        Self {
            state: Arc::new(RwLock::new(VectorRuntimeState::default())),
            ready_mirror,
            rebuild_mirror: Arc::new(AtomicBool::new(false)),
            configured: Arc::new(AtomicBool::new(configured)),
            activation: Arc::new(tokio::sync::Notify::new()),
            identity_transition: Arc::new(AtomicBool::new(false)),
            #[cfg(test)]
            admission_pause: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// Adapts a detached pair from an existing public constructor, preserving
    /// those call sites while giving every consumer the same handle shape.
    pub fn from_legacy(
        embedding: EmbeddingRuntime,
        index: Option<QdrantIndex>,
        collection: impl Into<String>,
    ) -> Self {
        let runtime = Self::with_state(embedding.is_configured(), embedding.readiness_flag());
        if let Some(index) = index
            && let Some((provider, identity)) = embedding.snapshot()
        {
            let collection = collection.into();
            runtime.with_write_state(|state| {
                let generation = state.generation;
                state.snapshot = Some(VectorSnapshot {
                    embedding: provider,
                    identity,
                    index: Arc::new(index),
                    collection,
                    generation,
                });
                state.ready = true;
            });
        }
        runtime
    }

    /// The readiness mirror other components read. Written only by a committed
    /// transition of this handle.
    pub fn readiness_flag(&self) -> Arc<AtomicBool> {
        self.ready_mirror.clone()
    }

    /// The rebuild mirror other components read.
    pub fn rebuild_flag(&self) -> Arc<AtomicBool> {
        self.rebuild_mirror.clone()
    }

    pub fn current(&self) -> CurrentVectorState {
        let state = self.read();
        CurrentVectorState {
            generation: state.generation,
            ready: state.ready && !state.rebuilding && !state.requires_rebuild,
            rebuilding: state.rebuilding,
            snapshot: state.snapshot.clone(),
        }
    }

    pub fn is_ready(&self) -> bool {
        let state = self.read();
        state.ready && !state.rebuilding && !state.requires_rebuild
    }

    /// Whether the persisted vectors need an explicit rebuild before this
    /// runtime may report readiness.
    pub fn requires_rebuild(&self) -> bool {
        self.read().requires_rebuild
    }

    /// Sets the persisted-identity mismatch flag. A set flag withdraws readiness
    /// immediately and prevents any publication or restore from reopening it; a
    /// successful explicit rebuild or confirmed acknowledgment clears it.
    pub fn set_requires_rebuild(&self, required: bool) {
        self.with_write_state(|state| {
            state.requires_rebuild = required;
            if required {
                state.ready = false;
            }
        });
    }

    /// The fingerprint of the currently configured identity, if known.
    pub fn configured_fingerprint(&self) -> Option<String> {
        self.read().configured_fingerprint.clone()
    }

    /// Records the configured identity fingerprint for the manual rebuild's
    /// successful state write. It never affects readiness.
    pub fn set_configured_fingerprint(&self, fingerprint: Option<String>) {
        self.with_write_state(|state| {
            state.configured_fingerprint = fingerprint;
        });
    }

    pub fn is_rebuilding(&self) -> bool {
        self.read().rebuilding
    }

    pub fn generation(&self) -> u64 {
        self.read().generation
    }

    pub fn is_configured(&self) -> bool {
        self.configured.load(Ordering::Acquire)
    }

    pub fn mark_configured(&self, configured: bool) {
        self.configured.store(configured, Ordering::Release);
        if configured {
            self.activation.notify_waiters();
        }
    }

    /// Resolves once configuration has been supplied, so a supervisor on an
    /// instance that started unconfigured wakes on the first valid save instead
    /// of polling.
    pub async fn wait_for_configuration(&self) {
        loop {
            let notified = self.activation.notified();
            if self.is_configured() {
                return;
            }
            notified.await;
        }
    }

    pub fn snapshot(&self) -> Option<VectorSnapshot> {
        self.read().snapshot.clone()
    }

    pub fn failure(&self) -> Option<VectorFailure> {
        self.read().failure.clone()
    }

    /// Starts a new generation, invalidating every in-flight attempt. The
    /// current pair is retained for diagnostics, but nothing is ready until a
    /// publisher for the new generation installs a candidate.
    pub fn begin_generation(&self) -> u64 {
        self.with_write_state(|state| {
            state.generation += 1;
            state.rebuilding = false;
            state.ready = false;
            state.failure = None;
            state.generation
        })
    }

    /// Starts an explicit rebuild fence. The pair is retained and re-stamped
    /// with the fence generation, so a successful completion reopens exactly
    /// that pair; no vector operation is ready until then.
    pub fn begin_rebuild(&self) -> u64 {
        self.with_write_state(|state| {
            state.generation += 1;
            if let Some(snapshot) = state.snapshot.as_mut() {
                snapshot.generation = state.generation;
            }
            state.rebuilding = true;
            state.ready = false;
            state.failure = None;
            state.generation
        })
    }

    /// Whether an identity transition currently owns the shared exclusion.
    pub fn is_transitioning(&self) -> bool {
        self.identity_transition.load(Ordering::Acquire)
    }

    /// Applies an admitted reconfiguration while holding the state write lock.
    ///
    /// `update` mutates the reconnect source and reports whether the
    /// vector-relevant configuration changed; the configured fingerprint is
    /// recorded in the same transition. A changed configuration advances the
    /// generation and withdraws readiness, so a snapshot published before it
    /// becomes stale and nothing observes a half-applied configuration. The
    /// caller holds the shared exclusion, so no rebuild start or publication can
    /// interleave with the commit.
    ///
    /// Returns `None` without running `update` when a rebuild fence is open
    /// under the same state lock: an attempt owns that generation and its
    /// collection, so a configuration apply must not advance or fence it. The
    /// caller releases the exclusion and defers.
    pub fn apply_runtime_reconfiguration(
        &self,
        fingerprint: Option<String>,
        update: impl FnOnce() -> bool,
    ) -> Option<(bool, u64)> {
        self.with_write_state(|state| {
            if state.rebuilding {
                return None;
            }
            let changed = update();
            state.configured_fingerprint = fingerprint;
            if changed {
                state.generation += 1;
                state.ready = false;
                state.failure = None;
            }
            Some((changed, state.generation))
        })
    }

    /// Acquires the shared exclusion for settling an admitted rebuild.
    ///
    /// Unlike [`Self::try_begin_identity_transition`] this does not refuse while
    /// a rebuild fence is open: the settling attempt owns that fence and
    /// validates its generation before writing. It still excludes a concurrent
    /// reconfiguration or identity transition.
    pub fn try_begin_rebuild_settlement(&self) -> bool {
        self.identity_transition
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    /// Acquires the shared identity-transition exclusion for a committed apply,
    /// refusing when a manual rebuild already owns the runtime.
    ///
    /// The exclusion is reserved first and rebuild ownership is inspected while
    /// it is held. That ordering is the contract: a manual rebuild start opens
    /// its fence only under the same exclusion, so a reservation that observes
    /// no fence cannot be overtaken by a start, and a reservation that observes
    /// an open fence releases and refuses instead of applying under another
    /// attempt's generation.
    pub fn try_begin_identity_transition(&self) -> bool {
        #[cfg(test)]
        self.run_admission_pause();
        if self
            .identity_transition
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return false;
        }
        if self.is_rebuilding() {
            self.identity_transition.store(false, Ordering::Release);
            return false;
        }
        true
    }

    #[cfg(test)]
    fn run_admission_pause(&self) {
        let pause = self
            .admission_pause
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(pause) = pause {
            pause();
        }
    }

    /// Installs a one-shot pause before the exclusion is reserved. Tests use it
    /// to hold a refresh at the pre-reservation point while a manual rebuild
    /// wins.
    #[cfg(test)]
    pub(crate) fn set_admission_pause(&self, pause: impl FnOnce() + Send + 'static) {
        *self
            .admission_pause
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Box::new(pause));
    }

    pub fn end_identity_transition(&self) {
        self.identity_transition.store(false, Ordering::Release);
    }

    /// Starts a manual rebuild from the runtime's own admitted context.
    pub fn try_begin_rebuild(&self) -> Option<u64> {
        self.try_admit_rebuild()
            .ok()
            .map(|admitted| admitted.generation)
    }

    /// Admits an explicit rebuild as one atomic decision over the runtime's own
    /// state.
    ///
    /// Under the shared exclusion and one state write lock, the published
    /// snapshot, its generation, its identity, and the configured fingerprint
    /// are validated and captured together, and only then is the rebuild
    /// generation opened and stamped. A retained snapshot whose generation is
    /// not the configured one is rejected instead of being relabelled, so a
    /// destination change that advanced the generation without publishing a new
    /// pair can never be rebuilt as the old collection under the new identity.
    /// A genuinely unknown populated collection at the current generation
    /// remains admissible.
    pub fn try_admit_rebuild(&self) -> std::result::Result<AdmittedRebuild, RebuildAdmissionError> {
        if !self.try_begin_identity_transition() {
            return Err(RebuildAdmissionError::Busy);
        }
        let admitted = self.with_write_state(|state| {
            if state.rebuilding {
                return Err(RebuildAdmissionError::Busy);
            }
            let Some(snapshot) = state.snapshot.clone() else {
                return Err(RebuildAdmissionError::NotConfigured);
            };
            if snapshot.generation != state.generation {
                return Err(RebuildAdmissionError::StaleSnapshot);
            }
            let Some(identity) = snapshot.identity.as_ref() else {
                return Err(RebuildAdmissionError::MissingIdentity);
            };
            if identity.dimensions == 0 || snapshot.collection.trim().is_empty() {
                return Err(RebuildAdmissionError::MissingIdentity);
            }
            let Some(fingerprint) = state.configured_fingerprint.clone() else {
                return Err(RebuildAdmissionError::MissingFingerprint);
            };
            state.generation += 1;
            if let Some(snapshot) = state.snapshot.as_mut() {
                snapshot.generation = state.generation;
            }
            state.rebuilding = true;
            state.ready = false;
            state.failure = None;
            Ok(AdmittedRebuild {
                generation: state.generation,
                snapshot: VectorSnapshot {
                    generation: state.generation,
                    ..snapshot
                },
                configured_fingerprint: fingerprint,
            })
        });
        self.end_identity_transition();
        admitted
    }

    /// Rewrites the published pair's provider and identity in place for a
    /// confirmed same-space transition. The Qdrant handle, collection, and
    /// generation are preserved, so requests holding an older snapshot finish on
    /// the provider they started with while new requests see one consistent new
    /// pair. A different dimension, an open rebuild fence, or no published pair
    /// is rejected and changes nothing.
    pub fn rekey_same_space(
        &self,
        provider: Arc<dyn EmbeddingProvider>,
        identity: EmbeddingIdentity,
    ) -> bool {
        self.with_write_state(|state| {
            let Some(snapshot) = state.snapshot.as_mut() else {
                return false;
            };
            if state.rebuilding {
                return false;
            }
            if snapshot.identity.as_ref().map(|current| current.dimensions)
                != Some(identity.dimensions)
            {
                return false;
            }
            snapshot.embedding = provider;
            snapshot.identity = Some(identity);
            true
        })
    }

    /// Withdraws readiness without changing the pair or generation, used when a
    /// committed transition could not be applied and the runtime must stop
    /// serving until reconciliation.
    pub fn withdraw_readiness(&self) {
        self.commit_readiness(false);
    }

    /// Completes the rebuild started at `generation`. A newer generation wins,
    /// and a stale completion mutates nothing.
    pub fn complete_rebuild(&self, generation: u64) -> bool {
        self.with_write_state(|state| {
            if state.generation != generation {
                return false;
            }
            state.rebuilding = false;
            state.ready = state
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.generation == generation);
            true
        })
    }

    /// Atomically settles a successful explicit rebuild that still owns
    /// `generation`.
    ///
    /// Clearing the requirement and completing the fence are one transition, so
    /// a superseded completion cannot clear a newer requirement or reopen a
    /// newer pair. Returns `false` with no mutation when the generation has
    /// moved on or no pair of that generation is published.
    pub fn complete_rebuild_success(&self, generation: u64) -> bool {
        self.with_write_state(|state| {
            if state.generation != generation {
                return false;
            }
            if !state
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.generation == generation)
            {
                return false;
            }
            state.requires_rebuild = false;
            state.rebuilding = false;
            state.ready = true;
            true
        })
    }

    /// Whether `generation` is still the runtime's current one.
    pub fn is_current_generation(&self, generation: u64) -> bool {
        self.read().generation == generation
    }

    /// Closes the rebuild fence for `generation` without reopening readiness.
    ///
    /// A failed explicit rebuild always latches the rebuild requirement for the
    /// current generation, whatever its starting value: the requested
    /// re-embedding did not complete, so a later liveness probe must not be able
    /// to reopen vector traffic. Only an explicit successful rebuild or a
    /// confirmed reconciliation clears the latch. A stale generation mutates
    /// nothing.
    pub fn fail_rebuild(&self, generation: u64) -> bool {
        self.with_write_state(|state| {
            if state.generation != generation {
                return false;
            }
            state.rebuilding = false;
            state.ready = false;
            state.requires_rebuild = true;
            true
        })
    }

    /// Restores readiness for the pair published under `generation` after a
    /// successful probe. A stale generation, a rebuild fence, or a different
    /// pair cannot reopen it.
    pub fn restore_ready(&self, generation: u64) -> bool {
        self.with_write_state(|state| {
            if state.generation != generation || state.rebuilding || state.requires_rebuild {
                return false;
            }
            if !state
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.generation == generation)
            {
                return false;
            }
            state.ready = true;
            true
        })
    }

    /// Installs `snapshot` only when `generation` is still current. A stale
    /// attempt cannot touch the published pair, the failure, or readiness.
    pub fn publish(&self, generation: u64, snapshot: VectorSnapshot) -> bool {
        self.with_write_state(|state| {
            // A recovery publication must not replace the pair while a
            // confirmed identity transition owns the exclusion; the supervisor
            // retries after the transition settles.
            if state.generation != generation || self.is_transitioning() {
                return false;
            }
            state.snapshot = Some(VectorSnapshot {
                generation,
                ..snapshot
            });
            state.failure = None;
            state.ready = !state.rebuilding && !state.requires_rebuild;
            true
        })
    }

    /// Records a failure for `generation` only.
    pub fn publish_failure(
        &self,
        generation: u64,
        class: VectorFailureClass,
        message: impl Into<String>,
    ) -> bool {
        self.with_write_state(|state| {
            if state.generation != generation {
                return false;
            }
            state.failure = Some(VectorFailure {
                generation,
                class,
                message: message.into(),
            });
            state.ready = false;
            true
        })
    }

    /// Commits an application-level readiness change. Ready is only admitted for
    /// a published pair that is not fenced by a rebuild.
    pub fn commit_readiness(&self, ready: bool) {
        self.with_write_state(|state| {
            state.ready =
                ready && !state.rebuilding && !state.requires_rebuild && state.snapshot.is_some();
        });
    }

    /// Replaces the provider for an identity-preserving credential or timeout
    /// change. The identity must match the published pair exactly; anything else
    /// is rejected so the collection and its vectors cannot be mixed.
    pub fn replace_provider(
        &self,
        provider: Arc<dyn EmbeddingProvider>,
        identity: EmbeddingIdentity,
    ) -> bool {
        self.with_write_state(|state| {
            let Some(snapshot) = state.snapshot.as_mut() else {
                return false;
            };
            if snapshot.identity.as_ref() != Some(&identity) {
                return false;
            }
            snapshot.embedding = provider;
            true
        })
    }

    /// A stable cache identity even while the handle is unready, so cached
    /// query vectors are never shared across vector spaces.
    pub fn cache_identity(&self) -> String {
        self.snapshot()
            .map(|snapshot| snapshot.cache_identity())
            .unwrap_or_else(|| "unconfigured".to_string())
    }

    /// A snapshot for work that may run unready, such as the rebuild itself.
    pub fn require_snapshot(&self) -> Result<VectorSnapshot> {
        let snapshot = self.read().snapshot.clone();
        snapshot.ok_or_else(|| self.unavailable_error())
    }

    /// A ready snapshot for new vector work. The rebuild fence, readiness, and
    /// the published pair are read in one lock acquisition, so the returned
    /// snapshot always belongs to the ready generation.
    pub fn require_ready(&self) -> Result<VectorSnapshot> {
        let current = self.current();
        if current.rebuilding {
            return Err(DomainError::unavailable(
                "vector index is rebuilding or unavailable; retry after the rebuild completes",
            )
            .into());
        }
        if !current.ready {
            return Err(DomainError::unavailable(
                "embedding/vector runtime is unavailable: vector index is unavailable; retry after it recovers",
            )
            .into());
        }
        current.snapshot.ok_or_else(|| self.unavailable_error())
    }

    fn unavailable_error(&self) -> anyhow::Error {
        if self.is_configured() {
            DomainError::unavailable("embedding/vector runtime is unavailable").into()
        } else {
            DomainError::unavailable("embedding/vector runtime is not configured").into()
        }
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, VectorRuntimeState> {
        self.state
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Runs one committed transition under the write lock and then refreshes the
    /// external mirrors from that committed state.
    fn with_write_state<T>(&self, transition: impl FnOnce(&mut VectorRuntimeState) -> T) -> T {
        let mut state = self
            .state
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let outcome = transition(&mut state);
        self.ready_mirror
            .store(state.ready && !state.rebuilding, Ordering::Release);
        self.rebuild_mirror
            .store(state.rebuilding, Ordering::Release);
        outcome
    }
}

/// What a collection without a recorded identity may be used for.
///
/// The legacy `recreate_on_dimension_mismatch` setting is deliberately not an
/// input: no value of it can authorize deleting, recreating, or adopting
/// existing vectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnrecordedCollection {
    /// No collection exists: safe zero-data bootstrap may create it.
    CreateMissing,
    /// The collection exists and is genuinely empty at the configured
    /// dimension: adopt it and record a zero-chunk identity.
    AdoptEmpty,
    /// The collection holds points at the configured dimension but has no
    /// recorded identity. It is never adopted or re-embedded implicitly, but a
    /// read-only index is published so the explicit manual rebuild can run.
    RebuildOnly { points: u64 },
    /// The collection declares a different dimension. Distinct spaces stay
    /// blocked instead of pretending to be supported.
    BlockedDimensions { found: usize, points: u64 },
}

pub(crate) fn decide_unrecorded_collection(
    existing: Option<(u64, usize)>,
    dimensions: usize,
) -> UnrecordedCollection {
    match existing {
        None => UnrecordedCollection::CreateMissing,
        Some((0, found)) if found == dimensions => UnrecordedCollection::AdoptEmpty,
        Some((points, found)) if found != dimensions => {
            UnrecordedCollection::BlockedDimensions { found, points }
        }
        Some((points, _)) => UnrecordedCollection::RebuildOnly { points },
    }
}

/// The index and the decision for a collection that has no recorded identity.
pub(crate) struct UnrecordedAttach {
    pub(crate) decision: UnrecordedCollection,
    /// Present for the create, empty-adopt, and rebuild-only paths; absent when
    /// the collection is blocked.
    pub(crate) index: Option<Arc<QdrantIndex>>,
}

/// Inspects a collection without a recorded identity and attaches it read-only
/// when it may be used.
///
/// Creation happens only for an absent collection. An existing collection is
/// never deleted or recreated, and an unknown populated one yields a
/// rebuild-only index: its vectors are never adopted as an empty space, no
/// identity is persisted here, and the caller decides whether the explicit
/// manual rebuild may run.
pub(crate) async fn attach_unrecorded_collection(
    config: &crate::config::QdrantConfig,
    dimensions: usize,
) -> Result<UnrecordedAttach> {
    let existing = QdrantIndex::inspect_existing(config, dimensions).await?;
    let decision = decide_unrecorded_collection(existing, dimensions);
    let index = match decision {
        UnrecordedCollection::CreateMissing => Some(Arc::new(
            QdrantIndex::bootstrap(config, dimensions, true).await?.0,
        )),
        UnrecordedCollection::AdoptEmpty => Some(Arc::new(
            QdrantIndex::bootstrap(config, dimensions, false).await?.0,
        )),
        UnrecordedCollection::RebuildOnly { .. } => Some(Arc::new(
            QdrantIndex::attach_read_only(config, dimensions).await?,
        )),
        UnrecordedCollection::BlockedDimensions { .. } => None,
    };
    Ok(UnrecordedAttach { decision, index })
}

/// Integration-test seam for the production unrecorded-collection bootstrap.
///
/// Compiled only with `integration-test-helpers`, so a loopback test can drive
/// the same read-only inspection, creation, and rebuild-only attach the
/// application connector uses without exposing it in a production build.
#[cfg(feature = "integration-test-helpers")]
#[doc(hidden)]
pub async fn attach_unrecorded_collection_for_test(
    config: &crate::config::QdrantConfig,
    dimensions: usize,
) -> Result<(bool, Option<Arc<QdrantIndex>>)> {
    let attached = attach_unrecorded_collection(config, dimensions).await?;
    let rebuild_only = matches!(attached.decision, UnrecordedCollection::RebuildOnly { .. });
    Ok((rebuild_only, attached.index))
}

/// Classifies a connection/bootstrap failure from its message without exposing
/// payloads or credentials. Unknown failures stay transient so a recoverable
/// outage is retried; an explicit permanent signal pauses the retry loop.
pub fn classify_failure(error: &anyhow::Error) -> VectorFailureClass {
    let message = error_message_chain(error);
    let is_permanent = [
        "dimension mismatch",
        "unauthorized",
        "unauthenticated",
        "forbidden",
        "permission denied",
        "access denied",
        "invalid argument",
        "invalid configuration",
        "configuration:",
        "collection is missing",
        "collection does not exist",
        "api key",
    ]
    .iter()
    .any(|signal| message.contains(signal));
    if is_permanent {
        VectorFailureClass::Configuration
    } else {
        VectorFailureClass::Transient
    }
}

fn error_message_chain(error: &anyhow::Error) -> String {
    let mut message = String::new();
    for cause in error.chain() {
        message.push_str(&cause.to_string().to_ascii_lowercase());
        message.push(' ');
    }
    message
}

#[cfg(test)]
#[path = "vector_runtime/tests.rs"]
mod tests;

#[cfg(test)]
mod test_support {
    use std::sync::Arc;

    use anyhow::Result;

    use crate::{
        embedding::{EmbeddingIdentity, EmbeddingProvider},
        qdrant_index::QdrantIndex,
        vector_runtime::{VectorRuntime, VectorSnapshot},
    };

    pub(crate) struct TestEmbeddingProvider;

    #[async_trait::async_trait]
    impl EmbeddingProvider for TestEmbeddingProvider {
        async fn embed_texts(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
            Ok(texts.iter().map(|_| vec![0.0; 4]).collect())
        }
    }

    impl VectorRuntime {
        pub(crate) fn for_test_with_index(index: QdrantIndex, identity: EmbeddingIdentity) -> Self {
            let runtime = Self::configured(Arc::new(std::sync::atomic::AtomicBool::new(false)));
            let generation = runtime.generation();
            runtime.publish(
                generation,
                VectorSnapshot {
                    embedding: Arc::new(TestEmbeddingProvider),
                    identity: Some(identity),
                    index: Arc::new(index),
                    collection: "test".to_string(),
                    generation,
                },
            );
            runtime
        }
    }
}
