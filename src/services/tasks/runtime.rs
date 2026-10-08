//! The lease heartbeat that keeps one claimed item alive.
//!
//! `runtime_driver.rs` owns what the worker does with a claim; this module owns
//! the periodic renewal that stops an abandoned item's lease from expiring, and
//! the guard that stops the heartbeat outliving the claim that started it.

use std::time::Duration;

use anyhow::Result;
use tokio::task::JoinHandle;
use uuid::Uuid;

use super::TaskService;

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(60);
const HEARTBEAT_MAX_CONSECUTIVE_ERRORS: u32 = 3;

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

async fn heartbeat_loop<F, Fut>(mut tick: F, interval_duration: Duration)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<bool>>,
{
    let mut interval = tokio::time::interval(interval_duration);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut consecutive_errors = 0u32;
    loop {
        interval.tick().await;
        match tick().await {
            Ok(true) => consecutive_errors = 0,
            Ok(false) => {
                // Lease lost (cancelled or reclaimed): stop renewing.
                break;
            }
            Err(_) => {
                consecutive_errors += 1;
                if consecutive_errors >= HEARTBEAT_MAX_CONSECUTIVE_ERRORS {
                    // Database unavailable: stop renewing so the lease expires
                    // and the item becomes recoverable by another worker.
                    break;
                }
            }
        }
    }
}

pub(super) fn spawn_item_heartbeat(
    service: TaskService,
    item_id: Uuid,
    lease_token: Uuid,
) -> HeartbeatGuard {
    HeartbeatGuard(tokio::spawn(heartbeat_loop(
        move || {
            let service = service.clone();
            async move { service.db().heartbeat_task_item(item_id, lease_token).await }
        },
        HEARTBEAT_INTERVAL,
    )))
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    };

    use super::{HEARTBEAT_MAX_CONSECUTIVE_ERRORS, heartbeat_loop};

    fn test_interval() -> std::time::Duration {
        std::time::Duration::from_millis(1)
    }

    #[tokio::test]
    async fn heartbeat_retries_transient_errors_before_stopping() {
        let ticks = Arc::new(AtomicU32::new(0));
        let inner = Arc::clone(&ticks);
        heartbeat_loop(
            move || {
                let ticks = Arc::clone(&inner);
                async move {
                    let count = ticks.fetch_add(1, Ordering::SeqCst) + 1;
                    if count < 3 {
                        Err(anyhow::anyhow!("database hiccup"))
                    } else {
                        Ok(false)
                    }
                }
            },
            test_interval(),
        )
        .await;
        assert_eq!(ticks.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn heartbeat_stops_after_max_consecutive_errors() {
        let ticks = Arc::new(AtomicU32::new(0));
        let inner = Arc::clone(&ticks);
        heartbeat_loop(
            move || {
                let ticks = Arc::clone(&inner);
                async move {
                    ticks.fetch_add(1, Ordering::SeqCst);
                    Err(anyhow::anyhow!("database down"))
                }
            },
            test_interval(),
        )
        .await;
        assert_eq!(
            ticks.load(Ordering::SeqCst),
            HEARTBEAT_MAX_CONSECUTIVE_ERRORS
        );
    }

    #[tokio::test]
    async fn heartbeat_stops_immediately_on_lease_loss() {
        let ticks = Arc::new(AtomicU32::new(0));
        let inner = Arc::clone(&ticks);
        heartbeat_loop(
            move || {
                let ticks = Arc::clone(&inner);
                async move {
                    ticks.fetch_add(1, Ordering::SeqCst);
                    Ok(false)
                }
            },
            test_interval(),
        )
        .await;
        assert_eq!(ticks.load(Ordering::SeqCst), 1);
    }
}
