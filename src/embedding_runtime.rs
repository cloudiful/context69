//! A process-wide, atomically replaceable handle to the embedding provider.
//!
//! The persisted runtime settings row (with its secret-store key) is the source
//! of truth; this handle is the in-memory projection that request paths read.
//! Clones share one cell, so installing a replacement is visible to every
//! consumer that holds a clone — the services do not carry their own snapshot.
//!
//! Concurrency contract:
//!
//! - a caller acquires the provider by cloning the `Arc` under a short read
//!   lock and releases the lock before any network I/O, so an in-flight request
//!   keeps the provider it started with;
//! - a replacement installs the provider and its identity together under one
//!   write lock, so no reader can observe a partially updated pair;
//! - a failed build never reaches the handle, so the previous usable runtime
//!   stays in place.
//!
//! The handle also carries the shared vector-index readiness flag, so an
//! embedding *writer* can refuse to start while an explicit startup/manual
//! rebuild is in progress. Writers and readers then observe the same gate: no
//! new work can add vectors to a collection that is being re-embedded.
//!
//! A settings save never changes the identity: while a fixed index is live, the
//! settings guard rejects model/dimension/endpoint changes before persistence,
//! and the reloader installs only credential/timeout changes (the identity it
//! installs is always the one already in effect).

use std::sync::{
    Arc, RwLock,
    atomic::{AtomicBool, Ordering},
};

use anyhow::Result;

use crate::{config::EmbeddingConfig, domain_errors::DomainError};

use super::{EmbeddingProvider, OpenAiCompatibleEmbeddingProvider};

/// The part of an embedding configuration that defines the vector space it
/// produces.
///
/// The API key and timeout are deliberately excluded: changing only those can
/// be applied live without touching the vector index. A change to any field
/// here means the collection's vectors are no longer homogeneous and must go
/// through the existing rebuild/readiness path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingIdentity {
    pub base_url: String,
    pub model: String,
    pub dimensions: usize,
}

impl EmbeddingIdentity {
    pub fn from_config(config: &EmbeddingConfig) -> Self {
        Self {
            base_url: config.base_url.trim_end_matches('/').to_string(),
            model: config.model.clone(),
            dimensions: config.dimensions,
        }
    }

    /// The identity a submitted settings row would install. The API key and
    /// timeout are intentionally absent: they are the only fields a live save
    /// may change.
    pub fn from_update(update: &crate::contracts::UpdateRuntimeEmbeddingSettings) -> Self {
        Self {
            base_url: update.base_url.trim().trim_end_matches('/').to_string(),
            model: update.model.trim().to_string(),
            dimensions: update.dimensions,
        }
    }
}

#[derive(Clone)]
struct EmbeddingRuntimeState {
    provider: Option<Arc<dyn EmbeddingProvider>>,
    identity: Option<EmbeddingIdentity>,
}

/// A shared handle to the embedding provider currently in effect.
#[derive(Clone)]
pub struct EmbeddingRuntime {
    state: Arc<RwLock<EmbeddingRuntimeState>>,
    read_ready: Arc<AtomicBool>,
}

impl Default for EmbeddingRuntime {
    fn default() -> Self {
        Self::new(None, None)
    }
}

impl EmbeddingRuntime {
    pub fn new(
        provider: Option<Arc<dyn EmbeddingProvider>>,
        identity: Option<EmbeddingIdentity>,
    ) -> Self {
        Self::with_readiness(provider, identity, Arc::new(AtomicBool::new(true)))
    }

    /// Builds a handle sharing an external readiness flag, so an embedding
    /// writer and the query path consult the same gate.
    pub fn with_readiness(
        provider: Option<Arc<dyn EmbeddingProvider>>,
        identity: Option<EmbeddingIdentity>,
        read_ready: Arc<AtomicBool>,
    ) -> Self {
        Self {
            state: Arc::new(RwLock::new(EmbeddingRuntimeState { provider, identity })),
            read_ready,
        }
    }

    /// The shared vector-index readiness flag. `false` while an
    /// identity-changing rebuild is in progress.
    pub fn readiness_flag(&self) -> Arc<AtomicBool> {
        self.read_ready.clone()
    }

    pub fn is_ready(&self) -> bool {
        self.read_ready.load(Ordering::Acquire)
    }

    /// The provider currently in effect, or `None` when embedding is not
    /// configured or not usable. The lock is released before this returns.
    pub fn current(&self) -> Option<Arc<dyn EmbeddingProvider>> {
        self.read().provider.clone()
    }

    /// The identity of the current provider, or `None` when none is installed.
    pub fn current_identity(&self) -> Option<EmbeddingIdentity> {
        self.read().identity.clone()
    }

    /// A stable key for the vector space the current provider produces, used to
    /// scope cached query vectors. It changes whenever the provider's identity
    /// changes, and falls back to the provider address for an identity-less
    /// handle so two different providers never share cached vectors.
    pub fn cache_identity(&self) -> String {
        let state = self.read();
        match (&state.identity, &state.provider) {
            (Some(identity), _) => format!(
                "{}|{}|{}",
                identity.base_url, identity.model, identity.dimensions
            ),
            (None, Some(provider)) => {
                format!("provider:{:p}", Arc::as_ptr(provider) as *const ())
            }
            (None, None) => "unconfigured".to_string(),
        }
    }

    pub fn is_configured(&self) -> bool {
        self.read().provider.is_some()
    }

    /// Atomically installs a new provider and its identity.
    pub fn install(&self, provider: Arc<dyn EmbeddingProvider>, identity: EmbeddingIdentity) {
        let mut state = self.write();
        state.provider = Some(provider);
        state.identity = Some(identity);
    }

    /// `current()` with the shared "not configured" error, for call sites that
    /// cannot proceed without a provider.
    pub fn require(&self) -> Result<Arc<dyn EmbeddingProvider>> {
        self.current().ok_or_else(|| {
            DomainError::unavailable("embedding runtime is not configured; save runtime settings")
                .into()
        })
    }

    /// `require()` that also refuses while the vector index is rebuilding, so a
    /// writer cannot add vectors with one identity to a collection being
    /// re-embedded with another. Embedders that perform the rebuild itself
    /// (which must run while unready) use [`Self::require`] instead.
    ///
    /// The message is worded so the existing dependency classifier treats it as
    /// a transient embedding/qdrant failure and retries with backoff rather than
    /// failing the task permanently.
    pub fn require_ready(&self) -> Result<Arc<dyn EmbeddingProvider>> {
        if !self.is_ready() {
            return Err(DomainError::unavailable(
                "embedding runtime is unavailable: vector index is rebuilding; retry after the rebuild completes",
            )
            .into());
        }
        self.require()
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, EmbeddingRuntimeState> {
        self.state
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, EmbeddingRuntimeState> {
        self.state
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl From<Option<Arc<dyn EmbeddingProvider>>> for EmbeddingRuntime {
    /// Adapts legacy construction (a bare provider option) to the handle. The
    /// runtime has no identity, so it never participates in identity-based
    /// replacement decisions.
    fn from(provider: Option<Arc<dyn EmbeddingProvider>>) -> Self {
        Self::new(provider, None)
    }
}

/// Builds a provider from configuration.
///
/// Construction performs no network I/O, so a failed build leaves the caller's
/// current runtime untouched.
pub fn build_provider(config: &EmbeddingConfig) -> Result<Arc<dyn EmbeddingProvider>> {
    Ok(Arc::new(OpenAiCompatibleEmbeddingProvider::new(
        config.clone(),
    )?))
}

/// Builds the provider for the settings connection probe from submitted values.
///
/// Field validation lives here so the endpoint and its tests share one set of
/// rules. Like [`build_provider`] it performs no network I/O; the caller embeds
/// one synthetic string to exercise the endpoint. No error path carries the API
/// key.
pub fn build_probe_provider(
    base_url: &str,
    model: &str,
    dimensions: usize,
    timeout_secs: u64,
    api_key: Option<String>,
) -> Result<Arc<dyn EmbeddingProvider>> {
    let base_url = base_url.trim();
    if base_url.is_empty() {
        return Err(
            DomainError::invalid_argument("runtime.embedding.base_url must not be empty").into(),
        );
    }
    let model = model.trim();
    if model.is_empty() {
        return Err(
            DomainError::invalid_argument("runtime.embedding.model must not be empty").into(),
        );
    }
    if dimensions == 0 {
        return Err(DomainError::invalid_argument(
            "runtime.embedding.dimensions must be greater than 0",
        )
        .into());
    }
    if timeout_secs == 0 {
        return Err(DomainError::invalid_argument(
            "runtime.embedding.timeout_secs must be greater than 0",
        )
        .into());
    }
    build_provider(&EmbeddingConfig {
        base_url: base_url.to_string(),
        api_key,
        model: model.to_string(),
        dimensions,
        timeout: std::time::Duration::from_secs(timeout_secs),
    })
}

#[cfg(test)]
#[path = "embedding_runtime_test_fixtures.rs"]
mod test_fixtures;

#[cfg(test)]
#[path = "embedding_runtime_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "embedding_runtime_readiness_tests.rs"]
mod readiness_tests;

#[cfg(test)]
#[path = "embedding_runtime_probe_tests.rs"]
mod probe_tests;
