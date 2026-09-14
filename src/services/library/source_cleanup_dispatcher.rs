//! Wakeable source-cleanup dispatcher (issue 389).
//!
//! The durable outbox (`library_storage_object_cleanup`) is the source of
//! truth: every release commits its intent in the same transaction as
//! `source_released_at`. This dispatcher only decides *when* to drain that
//! outbox. It never changes the safety order (identity lock, reference
//! check, S3 delete, row delete) and never waits for physical S3 deletion
//! inside the release request.
//!
//! Schedule:
//! - startup drain once, so a crash between commit and wake loses nothing;
//! - immediate drain after [`SourceCleanupDispatcher::wake`], which release
//!   call sites invoke only after their transaction commits;
//! - low-frequency fallback drain every [`SOURCE_CLEANUP_FALLBACK_INTERVAL`]
//!   (5 minutes) when no wake arrives;
//! - shutdown via [`CancellationToken`].
//!
//! A single intent failure never blocks later intents: each intent runs in
//! its own transaction inside `run_source_object_cleanup`, and a drain
//! failure only logs and continues.

#[cfg(test)]
use std::future::Future;
use std::{sync::Arc, time::Duration};

use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

/// Low-frequency safety net when no wake arrives. Manual and auto releases
/// wake the dispatcher immediately after commit, so this only fires for
/// crash recovery and missed wakes.
pub const SOURCE_CLEANUP_FALLBACK_INTERVAL: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DrainReason {
    Startup,
    Woken,
    Fallback,
}

impl DrainReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::Startup => "startup",
            Self::Woken => "woken",
            Self::Fallback => "fallback",
        }
    }
}

/// Process-level shared dispatcher. Clones share one [`Notify`], so every
/// release call site wakes the same background loop.
#[derive(Clone, Debug)]
pub struct SourceCleanupDispatcher {
    wake: Arc<Notify>,
    fallback_interval: Duration,
}

impl Default for SourceCleanupDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl SourceCleanupDispatcher {
    /// Shared dispatcher with the production 5-minute fallback.
    pub fn new() -> Self {
        Self {
            wake: Arc::new(Notify::new()),
            fallback_interval: SOURCE_CLEANUP_FALLBACK_INTERVAL,
        }
    }

    /// Test-only hook with a custom fallback: same shared-notify semantics
    /// as [`Self::new`]. Hidden from docs and never for production tuning;
    /// it stays `pub` only because integration tests under `tests/` are an
    /// external crate and cannot reach `pub(crate)`.
    #[doc(hidden)]
    pub fn with_fallback_interval(fallback_interval: Duration) -> Self {
        Self {
            wake: Arc::new(Notify::new()),
            fallback_interval: fallback_interval.max(Duration::from_millis(1)),
        }
    }

    /// Wake the background loop after a release transaction commits. Cheap
    /// and coalescing: concurrent wakes collapse into one extra drain.
    /// Never call before commit; a rollback must not wake.
    pub fn wake(&self) {
        self.wake.notify_one();
    }

    /// Whether two handles share the same underlying [`Notify`].
    #[cfg(test)]
    pub(crate) fn shares_wake_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.wake, &other.wake)
    }

    pub fn fallback_interval(&self) -> Duration {
        self.fallback_interval
    }

    /// Run until `shutdown` is cancelled. Starts with a startup drain, then
    /// drains on every wake or fallback tick.
    pub async fn run(&self, library: super::LibraryService, shutdown: CancellationToken) {
        self.run_with_interval(library, shutdown, self.fallback_interval)
            .await;
    }

    async fn run_with_interval(
        &self,
        library: super::LibraryService,
        shutdown: CancellationToken,
        fallback: Duration,
    ) {
        self.drain_once(&library, DrainReason::Startup).await;
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => break,
                _ = self.wake.notified() => {
                    self.drain_once(&library, DrainReason::Woken).await;
                }
                _ = tokio::time::sleep(fallback) => {
                    self.drain_once(&library, DrainReason::Fallback).await;
                }
            }
        }
    }

    /// Test seam: drive the wake/fallback/shutdown loop with an injected
    /// drain so unit tests need no database or storage.
    #[cfg(test)]
    pub(crate) async fn run_with_drain<F, Fut>(
        &self,
        shutdown: CancellationToken,
        fallback: Duration,
        drain: F,
    ) where
        F: Fn() -> Fut,
        Fut: Future<Output = anyhow::Result<()>>,
    {
        if let Err(error) = drain().await {
            tracing::warn!(%error, "source cleanup startup drain failed; retrying on wake/fallback");
        }
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => break,
                _ = self.wake.notified() => {
                    if let Err(error) = drain().await {
                        tracing::warn!(%error, "source cleanup woken drain failed; retrying on wake/fallback");
                    }
                }
                _ = tokio::time::sleep(fallback) => {
                    if let Err(error) = drain().await {
                        tracing::warn!(%error, "source cleanup fallback drain failed; retrying on wake/fallback");
                    }
                }
            }
        }
    }

    async fn drain_once(&self, library: &super::LibraryService, reason: DrainReason) {
        let started = std::time::Instant::now();
        let reason_str = reason.as_str();
        match reason {
            DrainReason::Startup => {
                tracing::info!(reason = reason_str, "cleanup_fallback_pass reason=startup");
            }
            DrainReason::Woken => {
                // Distinct from release-side `cleanup_woken after ...`,
                // which marks a wake being sent: this marks a dispatcher
                // drain starting after a wake.
                tracing::info!(reason = reason_str, "cleanup_drain_woken reason=woken");
            }
            DrainReason::Fallback => {
                tracing::info!(reason = reason_str, "cleanup_fallback_pass reason=fallback");
            }
        }
        match library
            .retry_pending_source_releases(super::DEFAULT_SOURCE_RELEASE_RETRY_BATCH_SIZE)
            .await
        {
            Ok(summary) if summary.released > 0 || summary.errors > 0 => {
                tracing::info!(
                    released = summary.released,
                    scanned = summary.scanned,
                    skipped_active = summary.skipped_active,
                    skipped_not_succeeded = summary.skipped_not_succeeded,
                    errors = summary.errors,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    reason = reason_str,
                    "pending source releases retried"
                );
            }
            Ok(summary) => {
                tracing::debug!(
                    released = summary.released,
                    scanned = summary.scanned,
                    skipped_active = summary.skipped_active,
                    skipped_not_succeeded = summary.skipped_not_succeeded,
                    errors = summary.errors,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    reason = reason_str,
                    "pending source releases retried without work"
                );
            }
            Err(error) => {
                tracing::warn!(
                    %error,
                    reason = reason_str,
                    "source release retry pass failed"
                );
            }
        }
        match library
            .run_source_object_cleanup(super::DEFAULT_SOURCE_OBJECT_CLEANUP_BATCH_SIZE)
            .await
        {
            Ok(summary) if summary.deleted > 0 || summary.failed > 0 || summary.cancelled > 0 => {
                tracing::info!(
                    deleted = summary.deleted,
                    cancelled = summary.cancelled,
                    failed = summary.failed,
                    scanned = summary.scanned,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    reason = reason_str,
                    "source object cleanup pass completed"
                );
            }
            Ok(summary) => {
                tracing::debug!(
                    deleted = summary.deleted,
                    cancelled = summary.cancelled,
                    failed = summary.failed,
                    scanned = summary.scanned,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    reason = reason_str,
                    "source object cleanup pass completed without work"
                );
            }
            Err(error) => {
                tracing::warn!(
                    %error,
                    reason = reason_str,
                    "source object cleanup pass failed"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use std::time::Duration;

    use tokio::time::timeout;
    use tokio_util::sync::CancellationToken;

    use super::SourceCleanupDispatcher;

    async fn wait_for_count(
        count: &Arc<AtomicUsize>,
        expected: usize,
        per_poll: Duration,
        polls: usize,
    ) -> bool {
        for _ in 0..polls {
            if count.load(Ordering::SeqCst) >= expected {
                return true;
            }
            tokio::time::sleep(per_poll).await;
        }
        count.load(Ordering::SeqCst) >= expected
    }

    #[test]
    fn clones_share_one_notify() {
        let dispatcher = SourceCleanupDispatcher::new();
        let cloned = dispatcher.clone();
        assert!(
            dispatcher.shares_wake_with(&cloned),
            "dispatcher clones must share a single Notify"
        );
        let other = SourceCleanupDispatcher::new();
        assert!(
            !dispatcher.shares_wake_with(&other),
            "separate constructors must not share Notify"
        );
    }

    #[test]
    fn default_fallback_is_five_minutes() {
        assert_eq!(
            SourceCleanupDispatcher::new().fallback_interval(),
            Duration::from_secs(5 * 60)
        );
        assert_eq!(
            super::SOURCE_CLEANUP_FALLBACK_INTERVAL,
            Duration::from_secs(5 * 60)
        );
    }

    #[tokio::test]
    async fn wake_triggers_immediate_drain() {
        let dispatcher = SourceCleanupDispatcher::with_fallback_interval(Duration::from_secs(3600));
        let count = Arc::new(AtomicUsize::new(0));
        let shutdown = CancellationToken::new();
        let runner_shutdown = shutdown.clone();
        let runner_dispatcher = dispatcher.clone();
        let runner_count = Arc::clone(&count);
        let handle = tokio::spawn(async move {
            runner_dispatcher
                .run_with_drain(runner_shutdown, Duration::from_secs(3600), move || {
                    let count = Arc::clone(&runner_count);
                    async move {
                        count.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    }
                })
                .await;
        });
        assert!(
            wait_for_count(&count, 1, Duration::from_millis(10), 100).await,
            "startup drain must run once"
        );
        assert_eq!(count.load(Ordering::SeqCst), 1);
        // No fallback may fire with an hour-long interval.
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "dispatcher must not poll at high frequency without wake"
        );
        dispatcher.wake();
        assert!(
            wait_for_count(&count, 2, Duration::from_millis(10), 100).await,
            "wake must trigger an immediate drain"
        );
        dispatcher.wake();
        assert!(
            wait_for_count(&count, 3, Duration::from_millis(10), 100).await,
            "second wake must trigger another drain"
        );
        shutdown.cancel();
        timeout(Duration::from_secs(2), handle)
            .await
            .expect("shutdown must stop the loop")
            .expect("dispatcher task");
    }

    #[tokio::test]
    async fn fallback_triggers_without_wake() {
        let dispatcher = SourceCleanupDispatcher::with_fallback_interval(Duration::from_millis(20));
        let count = Arc::new(AtomicUsize::new(0));
        let shutdown = CancellationToken::new();
        let runner_shutdown = shutdown.clone();
        let runner_dispatcher = dispatcher.clone();
        let runner_count = Arc::clone(&count);
        let handle = tokio::spawn(async move {
            runner_dispatcher
                .run_with_drain(runner_shutdown, Duration::from_millis(20), move || {
                    let count = Arc::clone(&runner_count);
                    async move {
                        count.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    }
                })
                .await;
        });
        assert!(
            wait_for_count(&count, 3, Duration::from_millis(10), 200).await,
            "fallback must drain repeatedly without any wake"
        );
        shutdown.cancel();
        timeout(Duration::from_secs(2), handle)
            .await
            .expect("shutdown must stop the fallback loop")
            .expect("dispatcher task");
    }

    #[tokio::test]
    async fn shutdown_exits_promptly_when_idle() {
        let dispatcher = SourceCleanupDispatcher::with_fallback_interval(Duration::from_secs(3600));
        let shutdown = CancellationToken::new();
        let runner_shutdown = shutdown.clone();
        let runner_dispatcher = dispatcher.clone();
        let handle = tokio::spawn(async move {
            runner_dispatcher
                .run_with_drain(runner_shutdown, Duration::from_secs(3600), || async {
                    Ok(())
                })
                .await;
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        shutdown.cancel();
        timeout(Duration::from_secs(2), handle)
            .await
            .expect("shutdown must exit an idle dispatcher")
            .expect("dispatcher task");
    }

    #[tokio::test]
    async fn failed_drain_does_not_block_later_wakes() {
        let dispatcher = SourceCleanupDispatcher::with_fallback_interval(Duration::from_secs(3600));
        let calls = Arc::new(AtomicUsize::new(0));
        let shutdown = CancellationToken::new();
        let runner_shutdown = shutdown.clone();
        let runner_dispatcher = dispatcher.clone();
        let runner_calls = Arc::clone(&calls);
        let handle = tokio::spawn(async move {
            runner_dispatcher
                .run_with_drain(runner_shutdown, Duration::from_secs(3600), move || {
                    let calls = Arc::clone(&runner_calls);
                    async move {
                        let call = calls.fetch_add(1, Ordering::SeqCst) + 1;
                        if call == 2 {
                            anyhow::bail!("injected drain failure");
                        }
                        Ok(())
                    }
                })
                .await;
        });
        assert!(
            wait_for_count(&calls, 1, Duration::from_millis(10), 100).await,
            "startup drain must run"
        );
        dispatcher.wake();
        assert!(
            wait_for_count(&calls, 2, Duration::from_millis(10), 100).await,
            "failing woken drain must still be attempted"
        );
        dispatcher.wake();
        assert!(
            wait_for_count(&calls, 3, Duration::from_millis(10), 100).await,
            "a failed drain must not block the next wake"
        );
        shutdown.cancel();
        timeout(Duration::from_secs(2), handle)
            .await
            .expect("shutdown must exit after failures")
            .expect("dispatcher task");
    }
}
