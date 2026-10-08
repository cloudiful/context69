//! The dispatcher loop: decide when an item is claimed and when a lease
//! expires.
//!
//! The lifecycle transitions it emits go through `lifecycle_logging.rs`, which
//! owns the shared log vocabulary.

use std::time::Duration;

use tokio::time::{MissedTickBehavior, interval};

use super::TaskService;
use super::lifecycle_logging::{
    log_claim, log_claims_dispatched, log_maintenance_failed, log_maintenance_outcome,
    log_no_claimable_item,
};

const RECOVERY_INTERVAL: Duration = Duration::from_secs(30);

pub(super) fn start(service: &TaskService) {
    if !service.dispatcher_started() {
        return;
    }

    let service = service.clone();
    tokio::spawn(async move {
        // Startup path: run maintenance once so any exhausted items or
        // expired attempts left by the previous process converge before
        // the first claim, then drain the available queue.
        run_maintenance(&service).await;
        dispatch_available(&service).await;

        let mut recovery = interval(RECOVERY_INTERVAL);
        recovery.set_missed_tick_behavior(MissedTickBehavior::Delay);
        // The first interval tick completes immediately. Consume it so the
        // first periodic recovery runs 30 seconds after startup, not
        // milliseconds after the explicit startup maintenance above.
        recovery.tick().await;

        loop {
            tokio::select! {
                _ = service.dispatch_notify().notified() => {
                    // Notification-driven wake: only the fast claim. The
                    // maintenance UPDATE/RETURNING work is reserved for the
                    // periodic recovery tick so spammed submit/retry/finish
                    // notifications avoid the exhausted/expired work.
                    dispatch_available(&service).await;
                }
                _ = recovery.tick() => {
                    // Recovery tick: maintenance must converge exhausted
                    // items and expired attempts even when no row is
                    // claimable, then drain any newly eligible work.
                    // Sequential so the fast dispatch sees freshly
                    // converged state. Failure is logged, never propagated.
                    run_maintenance(&service).await;
                    dispatch_available(&service).await;
                }
            }
        }
    });
}

async fn dispatch_available(service: &TaskService) {
    let mut claimed_total = 0usize;
    // The durable parent-task capacity is global: every replica passes the same
    // configured `scheduler.max_concurrency`, and the database admits at most
    // that many parents across all replicas. The local semaphore only bounds
    // this replica's workers.
    let capacity = i64::try_from(service.worker_capacity()).unwrap_or(i64::MAX);
    loop {
        let available_slots = service.available_worker_slots();
        if available_slots == 0 {
            break;
        }
        // Hot path: only the admission claim statement runs (plus the
        // admission lock that serializes it across replicas). Maintenance
        // lives on the recovery tick so notification-driven wakes (submit,
        // retry, finish, worker release) avoid the exhausted/expired
        // UPDATE CTEs that used to run on every wake.
        let items = match service.db().claim_items_fast(capacity).await {
            Ok(items) => items,
            Err(error) => {
                tracing::warn!(%error, "failed to claim context69 task items");
                break;
            }
        };
        if items.is_empty() {
            log_no_claimable_item(service.worker_capacity(), available_slots);
            break;
        }
        for item in items {
            log_claim(&item);
            let Ok(permit) = service.worker_slots().try_acquire_owned() else {
                break;
            };
            service.spawn_item(item, permit);
            claimed_total += 1;
        }
    }

    log_claims_dispatched(
        claimed_total,
        service.worker_capacity(),
        service.available_worker_slots(),
    );
}

/// Run `Database::maintain_claim_state` and log (but never propagate)
/// failures. The recovery tick is best-effort: a transient DB error
/// must not stall the dispatcher loop, and the next tick will retry.
/// Maintenance is intentionally called sequentially before the recovery
/// dispatch call so the fast claim path sees freshly converged state
/// without running its own UPDATE/RETURNING work.
async fn run_maintenance(service: &TaskService) {
    match service.db().maintain_claim_state().await {
        Ok(outcome) => log_maintenance_outcome(&outcome),
        Err(error) => log_maintenance_failed(&error),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use futures::{StreamExt, stream::FuturesUnordered};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::{
        sync::{Notify, Semaphore},
        time::timeout,
    };

    #[tokio::test]
    async fn many_pending_tasks_never_create_more_workers_than_slots() {
        for capacity in [1, 2, 4] {
            let semaphore = Arc::new(Semaphore::new(capacity));
            let active = Arc::new(AtomicUsize::new(0));
            let peak = Arc::new(AtomicUsize::new(0));
            let mut workers = FuturesUnordered::new();
            let mut remaining = 10_000usize;

            while remaining > 0 || !workers.is_empty() {
                while remaining > 0 {
                    let Ok(permit) = semaphore.clone().try_acquire_owned() else {
                        break;
                    };
                    remaining -= 1;
                    let active = Arc::clone(&active);
                    let peak = Arc::clone(&peak);
                    workers.push(async move {
                        let current = active.fetch_add(1, Ordering::AcqRel) + 1;
                        peak.fetch_max(current, Ordering::AcqRel);
                        tokio::task::yield_now().await;
                        active.fetch_sub(1, Ordering::AcqRel);
                        drop(permit);
                    });
                }

                workers.next().await;
            }

            assert!(peak.load(Ordering::Acquire) <= capacity);
            assert_eq!(active.load(Ordering::Acquire), 0);
        }
    }

    #[tokio::test]
    async fn repeated_notifications_coalesce_without_waiting_workers() {
        let notify = Notify::new();
        for _ in 0..10_000 {
            notify.notify_one();
        }

        notify.notified().await;
        assert!(
            timeout(Duration::from_millis(10), notify.notified())
                .await
                .is_err()
        );
    }
}
