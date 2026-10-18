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

struct VectorRuntimeState {
    generation: u64,
    rebuilding: bool,
    ready: bool,
    snapshot: Option<VectorSnapshot>,
    failure: Option<VectorFailure>,
}

impl Default for VectorRuntimeState {
    fn default() -> Self {
        Self {
            generation: 1,
            rebuilding: false,
            ready: false,
            snapshot: None,
            failure: None,
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
            ready: state.ready && !state.rebuilding,
            rebuilding: state.rebuilding,
            snapshot: state.snapshot.clone(),
        }
    }

    pub fn is_ready(&self) -> bool {
        let state = self.read();
        state.ready && !state.rebuilding
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

    /// Closes the rebuild fence for `generation` without reopening readiness.
    pub fn fail_rebuild(&self, generation: u64) -> bool {
        self.with_write_state(|state| {
            if state.generation != generation {
                return false;
            }
            state.rebuilding = false;
            state.ready = false;
            true
        })
    }

    /// Restores readiness for the pair published under `generation` after a
    /// successful probe. A stale generation, a rebuild fence, or a different
    /// pair cannot reopen it.
    pub fn restore_ready(&self, generation: u64) -> bool {
        self.with_write_state(|state| {
            if state.generation != generation || state.rebuilding {
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
            if state.generation != generation {
                return false;
            }
            state.snapshot = Some(VectorSnapshot {
                generation,
                ..snapshot
            });
            state.failure = None;
            state.ready = !state.rebuilding;
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
            state.ready = ready && !state.rebuilding && state.snapshot.is_some();
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
