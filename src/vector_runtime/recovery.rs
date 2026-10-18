//! Bounded startup retry and the ongoing recovery supervisor for the shared
//! vector runtime.
//!
//! The engine is deterministic and independent of Wall-clock sleeps and of the
//! real Qdrant network: it drives a [`VectorConnector`] seam and a [`Sleeper`]
//! seam, so tests can inject delays, failures, and cancellation without a slow
//! real-time wait. The application wiring in `services::app::vector_recovery`
//! supplies the production connector (runtime settings + Qdrant attach) and
//! observer (dependency gates + standalone embedding handle).
//!
//! Invariants owned here:
//!
//! - every attempt captures the generation it started with and may only
//!   publish when that generation is still current, so a stale or cancelled
//!   candidate cannot install a client, clear a failure, or reopen readiness;
//! - no lock is held across an attach, probe, or sleep: the connector sees only
//!   owned `VectorPair`/`VectorSnapshot` values;
//! - the startup budget is separate from the ongoing probe cadence, and a
//!   permanent configuration/auth/dimension failure pauses the rapid retry
//!   until the configuration fingerprint changes.

use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use async_trait::async_trait;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::{
    embedding::{EmbeddingIdentity, EmbeddingProvider},
    qdrant_index::QdrantIndex,
    vector_runtime::{VectorFailureClass, VectorRuntime, VectorSnapshot, classify_failure},
};

pub const STARTUP_BUDGET: Duration = Duration::from_secs(30);
pub const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(5);
pub const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
pub const PROBE_INTERVAL: Duration = Duration::from_secs(10);
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const BACKOFF_CAP: Duration = Duration::from_secs(30);

/// A coherent provider/identity pair plus the collection it writes to, resolved
/// from the current configuration before an attach attempt.
pub struct VectorPair {
    pub embedding: Arc<dyn EmbeddingProvider>,
    pub identity: EmbeddingIdentity,
    pub collection: String,
}

/// Establishes and probes the vector runtime. The production implementation
/// performs only read-only attach/probe RPCs; creation is allowed only for a
/// genuinely uninitialized collection.
#[async_trait]
pub trait VectorConnector: Send + Sync {
    async fn pair(&self) -> Result<VectorPair>;
    async fn attach(&self, pair: &VectorPair) -> Result<Arc<QdrantIndex>>;
    async fn probe(&self, snapshot: &VectorSnapshot) -> Result<()>;
    fn configuration_fingerprint(&self) -> String;
}

/// Outcome notifications for the owning application. The engine calls these
/// only for outcomes whose generation is still current.
#[async_trait]
pub trait RecoveryObserver: Send + Sync {
    async fn published(&self, snapshot: &VectorSnapshot);
    async fn failed(&self, generation: u64, class: VectorFailureClass, message: &str);
    async fn probe_failed(&self, snapshot: &VectorSnapshot, message: &str);
    async fn probe_recovered(&self, snapshot: &VectorSnapshot);
}

#[async_trait]
pub trait Sleeper: Send + Sync {
    async fn sleep(&self, duration: Duration);
}

pub struct TokioSleeper;

#[async_trait]
impl Sleeper for TokioSleeper {
    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RecoveryPolicy {
    pub startup_budget: Duration,
    pub attempt_timeout: Duration,
    pub initial_backoff: Duration,
    pub backoff_cap: Duration,
    pub probe_interval: Duration,
    pub probe_timeout: Duration,
}

impl Default for RecoveryPolicy {
    fn default() -> Self {
        Self {
            startup_budget: STARTUP_BUDGET,
            attempt_timeout: ATTEMPT_TIMEOUT,
            initial_backoff: INITIAL_BACKOFF,
            backoff_cap: BACKOFF_CAP,
            probe_interval: PROBE_INTERVAL,
            probe_timeout: PROBE_TIMEOUT,
        }
    }
}

pub struct VectorRecovery {
    runtime: VectorRuntime,
    connector: Arc<dyn VectorConnector>,
    observer: Option<Arc<dyn RecoveryObserver>>,
    sleeper: Arc<dyn Sleeper>,
    policy: RecoveryPolicy,
}

impl VectorRecovery {
    pub fn new(
        runtime: VectorRuntime,
        connector: Arc<dyn VectorConnector>,
        sleeper: Arc<dyn Sleeper>,
    ) -> Self {
        Self {
            runtime,
            connector,
            observer: None,
            sleeper,
            policy: RecoveryPolicy::default(),
        }
    }

    pub fn with_observer(mut self, observer: Arc<dyn RecoveryObserver>) -> Self {
        self.observer = Some(observer);
        self
    }

    pub fn with_policy(mut self, policy: RecoveryPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Runs the bounded startup retry loop. Returns true when a client was
    /// published; a permanent failure or an exhausted budget returns false and
    /// leaves management/readiness available for the ongoing supervisor.
    pub async fn run_startup(&self, shutdown: &CancellationToken) -> bool {
        if !self.runtime.is_configured() {
            // Nothing is configured yet: never build a provider or touch an
            // endpoint for a dormant instance.
            return false;
        }
        let mut spent = Duration::ZERO;
        let mut attempt = 0u32;
        loop {
            if shutdown.is_cancelled() {
                return false;
            }
            let generation = self.runtime.generation();
            match self.attempt(generation, shutdown).await {
                Attempt::Connected => return true,
                Attempt::Stale => continue,
                Attempt::Cancelled => return false,
                Attempt::Failed { class } => {
                    if class == VectorFailureClass::Configuration {
                        return false;
                    }
                    if spent >= self.policy.startup_budget {
                        return false;
                    }
                    let remaining = self.policy.startup_budget - spent;
                    let delay = next_backoff(&self.policy, attempt).min(remaining);
                    spent = spent.saturating_add(delay);
                    attempt = attempt.saturating_add(1);
                    if !sleep_or_cancel(&*self.sleeper, delay, shutdown).await {
                        return false;
                    }
                }
            }
        }
    }

    /// The ongoing supervisor. It probes a published pair on the healthy
    /// cadence and retries the bootstrap when no pair is published. A
    /// permanent failure pauses the retry until the generation or the
    /// configuration fingerprint changes.
    pub async fn supervise(&self, shutdown: CancellationToken) {
        let mut attempt = 0u32;
        let mut paused: Option<(u64, String)> = None;
        loop {
            if shutdown.is_cancelled() {
                return;
            }
            if !self.runtime.is_configured() {
                // Dormant: the instance started without runtime settings, so no
                // provider or endpoint may be touched until one is committed.
                tokio::select! {
                    _ = shutdown.cancelled() => return,
                    _ = self.runtime.wait_for_configuration() => {}
                }
                continue;
            }
            if let Some((generation, fingerprint)) = &paused {
                if self.runtime.generation() == *generation
                    && self.connector.configuration_fingerprint() == *fingerprint
                {
                    if !sleep_or_cancel(&*self.sleeper, self.policy.probe_interval, &shutdown).await
                    {
                        return;
                    }
                    continue;
                }
                paused = None;
            }

            let current = self.runtime.current();
            if let Some(snapshot) = current.snapshot {
                if snapshot.generation != current.generation {
                    // A settings change superseded this pair; reconnect below
                    // rather than probing an obsolete collection.
                } else {
                    if !sleep_or_cancel(&*self.sleeper, self.policy.probe_interval, &shutdown).await
                    {
                        return;
                    }
                    // A rebuild or settings change invalidates this snapshot
                    // while the probe was sleeping.
                    if self.runtime.generation() != snapshot.generation {
                        continue;
                    }
                    match tokio::time::timeout(
                        self.policy.probe_timeout,
                        self.connector.probe(&snapshot),
                    )
                    .await
                    {
                        Ok(Ok(())) => {
                            if self.runtime.restore_ready(snapshot.generation)
                                && let Some(observer) = &self.observer
                            {
                                observer.probe_recovered(&snapshot).await;
                            }
                        }
                        Ok(Err(error)) => {
                            let message = sanitize_message(&error.to_string());
                            let class = classify_failure(&error);
                            if self.runtime.publish_failure(
                                snapshot.generation,
                                class,
                                message.clone(),
                            ) {
                                if class == VectorFailureClass::Configuration {
                                    paused = Some((
                                        self.runtime.generation(),
                                        self.connector.configuration_fingerprint(),
                                    ));
                                }
                                if let Some(observer) = &self.observer {
                                    observer.probe_failed(&snapshot, &message).await;
                                }
                            }
                        }
                        Err(_) => {
                            let message = "vector runtime probe timed out".to_string();
                            if self.runtime.publish_failure(
                                snapshot.generation,
                                VectorFailureClass::Transient,
                                message.clone(),
                            ) && let Some(observer) = &self.observer
                            {
                                observer.probe_failed(&snapshot, &message).await;
                            }
                        }
                    }
                    continue;
                }
            }

            let generation = self.runtime.generation();
            match self.attempt(generation, &shutdown).await {
                Attempt::Connected => attempt = 0,
                Attempt::Stale => continue,
                Attempt::Cancelled => return,
                Attempt::Failed { class } => {
                    if class == VectorFailureClass::Configuration {
                        paused = Some((
                            self.runtime.generation(),
                            self.connector.configuration_fingerprint(),
                        ));
                        continue;
                    }
                    let delay = next_backoff(&self.policy, attempt);
                    attempt = attempt.saturating_add(1);
                    if !sleep_or_cancel(&*self.sleeper, delay, &shutdown).await {
                        return;
                    }
                }
            }
        }
    }

    async fn attempt(&self, generation: u64, shutdown: &CancellationToken) -> Attempt {
        if shutdown.is_cancelled() {
            return Attempt::Cancelled;
        }
        let pair = match self.connector.pair().await {
            Ok(pair) => pair,
            Err(error) => {
                return self.fail(generation, error, shutdown).await;
            }
        };
        let attached =
            tokio::time::timeout(self.policy.attempt_timeout, self.connector.attach(&pair)).await;
        let index = match attached {
            Ok(Ok(index)) => index,
            Ok(Err(error)) => return self.fail(generation, error, shutdown).await,
            Err(_) => {
                let error = anyhow::anyhow!("vector runtime attach timed out");
                return self.fail(generation, error, shutdown).await;
            }
        };
        if shutdown.is_cancelled() {
            return Attempt::Cancelled;
        }
        let snapshot = VectorSnapshot {
            embedding: pair.embedding,
            identity: Some(pair.identity),
            index,
            collection: pair.collection,
            generation,
        };
        if !self.runtime.publish(generation, snapshot.clone()) {
            return Attempt::Stale;
        }
        if let Some(observer) = &self.observer {
            observer.published(&snapshot).await;
        }
        Attempt::Connected
    }

    async fn fail(
        &self,
        generation: u64,
        error: anyhow::Error,
        shutdown: &CancellationToken,
    ) -> Attempt {
        if shutdown.is_cancelled() {
            return Attempt::Cancelled;
        }
        let class = classify_failure(&error);
        let message = sanitize_message(&error.to_string());
        warn!(
            generation,
            class = ?class,
            error = %message,
            "vector runtime attempt failed"
        );
        if !self
            .runtime
            .publish_failure(generation, class, message.clone())
        {
            return Attempt::Stale;
        }
        if let Some(observer) = &self.observer {
            observer.failed(generation, class, &message).await;
        }
        Attempt::Failed { class }
    }
}

enum Attempt {
    Connected,
    Stale,
    Cancelled,
    Failed { class: VectorFailureClass },
}

/// Truncates a diagnostic message and replaces any endpoint URL, so recovery
/// logs and gate rows never carry a credentialed URL or a full remote payload.
fn sanitize_message(message: &str) -> String {
    let redacted = message
        .split_whitespace()
        .map(|token| {
            if token.starts_with("http://") || token.starts_with("https://") {
                "<endpoint>"
            } else {
                token
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    if redacted.chars().count() > 500 {
        format!("{}...", redacted.chars().take(500).collect::<String>())
    } else {
        redacted
    }
}

async fn sleep_or_cancel(
    sleeper: &dyn Sleeper,
    duration: Duration,
    shutdown: &CancellationToken,
) -> bool {
    tokio::select! {
        _ = sleeper.sleep(duration) => !shutdown.is_cancelled(),
        _ = shutdown.cancelled() => false,
    }
}

/// 1s, 2s, 4s, 8s, then the configured cap, each with at most 20% jitter.
fn next_backoff(policy: &RecoveryPolicy, attempt: u32) -> Duration {
    // 1s, 2s, 4s, 8s, then the ongoing cap.
    let base = if attempt < 4 {
        policy.initial_backoff.saturating_mul(1u32 << attempt)
    } else {
        policy.backoff_cap
    }
    .min(policy.backoff_cap);
    base.mul_f64(jitter_fraction())
}

fn jitter_fraction() -> f64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.subsec_nanos() as u64)
        .unwrap_or(0);
    0.8 + (nanos % 4001) as f64 / 10000.0
}

#[cfg(test)]
#[path = "recovery_tests.rs"]
mod tests;
