//! The lease heartbeat that keeps one claimed item alive.
//!
//! `runtime_driver.rs` owns what the worker does with a claim; this module owns
//! the periodic renewal that stops an abandoned item's lease from expiring, and
//! the guard that stops the heartbeat outliving the claim that started it.
//!
//! The library owns the lease, so the renewal goes through it and the deadline
//! the domain mirrors is the one the library returned. A renewal that fails or
//! times out counts against the failure budget, and a worker that cannot keep
//! its claim drops the work it started instead of letting it run on a lease it
//! no longer holds.

use std::time::Duration;

use anyhow::Result;
use tokio::task::{AbortHandle, JoinHandle};

use super::TaskService;
use crate::db::ExecutionContext;

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(60);
const HEARTBEAT_MAX_CONSECUTIVE_ERRORS: u32 = 3;
const HEARTBEAT_RENEW_TIMEOUT: Duration = Duration::from_secs(30);

/// The renewal timing of the heartbeat.
fn renewal_timing() -> (Duration, Duration) {
    (HEARTBEAT_INTERVAL, HEARTBEAT_RENEW_TIMEOUT)
}

/// Aborts the wrapped heartbeat task when dropped so that early `?` returns
/// from `run_item` cannot leave an orphaned heartbeat renewing a lease forever.
pub(super) struct HeartbeatGuard(JoinHandle<()>);

impl HeartbeatGuard {
    pub(super) fn abort(&self) {
        self.0.abort();
    }
}

impl Drop for HeartbeatGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Renews the lease of the claimed item until it is lost.
///
/// The work the claim started is dropped as soon as the claim is gone: a
/// worker that lost its lease must not keep running the item it no longer
/// owns.
async fn heartbeat_loop<F, Fut>(
    execution: ExecutionContext,
    mut tick: F,
    interval_duration: Duration,
    abort_work: AbortHandle,
) where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<bool>>,
{
    let (_, renew_timeout) = renewal_timing();
    let mut interval = tokio::time::interval(interval_duration);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut consecutive_errors = 0u32;
    loop {
        interval.tick().await;
        match tokio::time::timeout(renew_timeout, tick()).await {
            Ok(Ok(true)) => consecutive_errors = 0,
            Ok(Ok(false)) => {
                // The lease is gone (cancelled, reclaimed or expired): the work
                // is dropped with it.
                tracing::warn!(
                    target: "task_lifecycle",
                    task_id = %execution.task_id(),
                    item_id = %execution.item_id,
                    attempt_id = %execution.attempt_id(),
                    "task item lease was lost; stopping the heartbeat and dropping its work"
                );
                abort_work.abort();
                break;
            }
            Ok(Err(error)) => {
                consecutive_errors += 1;
                tracing::warn!(
                    target: "task_lifecycle",
                    task_id = %execution.task_id(),
                    item_id = %execution.item_id,
                    attempt_id = %execution.attempt_id(),
                    consecutive_errors,
                    error = %error,
                    "task item lease renewal failed"
                );
                if consecutive_errors >= HEARTBEAT_MAX_CONSECUTIVE_ERRORS {
                    // Database unavailable: stop renewing so the lease expires
                    // and the item becomes recoverable by another worker. The
                    // work is dropped instead of running on a lease that is
                    // about to expire.
                    abort_work.abort();
                    break;
                }
            }
            Err(_) => {
                consecutive_errors += 1;
                tracing::warn!(
                    target: "task_lifecycle",
                    task_id = %execution.task_id(),
                    item_id = %execution.item_id,
                    attempt_id = %execution.attempt_id(),
                    consecutive_errors,
                    timeout_secs = renew_timeout.as_secs(),
                    "task item lease renewal timed out"
                );
                if consecutive_errors >= HEARTBEAT_MAX_CONSECUTIVE_ERRORS {
                    abort_work.abort();
                    break;
                }
            }
        }
    }
}

pub(super) fn spawn_item_heartbeat(
    service: TaskService,
    execution: ExecutionContext,
    abort_work: AbortHandle,
) -> HeartbeatGuard {
    let (interval, _) = renewal_timing();
    let identity = execution.clone();
    HeartbeatGuard(tokio::spawn(heartbeat_loop(
        identity,
        move || {
            let service = service.clone();
            let execution = execution.clone();
            async move { service.db().heartbeat_task_item(&execution).await }
        },
        interval,
        abort_work,
    )))
}
