//! The dispatcher loop: decide when an item is claimed and when a lease
//! expires.
//!
//! The lifecycle transitions it emits go through `lifecycle_logging.rs`, which
//! owns the shared log vocabulary.

use std::time::Duration;

use anyhow::Result;
use tokio::task::JoinHandle;
use tokio::time::{MissedTickBehavior, interval};
use tokio_util::sync::CancellationToken;

use super::TaskService;
use super::lifecycle_logging::{
    log_claim, log_claims_dispatched, log_maintenance_failed, log_maintenance_outcome,
    log_no_claimable_item,
};

const RECOVERY_INTERVAL: Duration = Duration::from_secs(30);

/// Owned completion of the dispatcher loop.
///
/// Awaiting it proves the loop can no longer claim work; it does not wait for
/// items that were already dispatched.
pub(crate) struct DispatcherHandle {
    completion: JoinHandle<()>,
}

impl DispatcherHandle {
    pub(super) async fn wait(self) {
        if let Err(error) = self.completion.await {
            tracing::warn!(%error, "task dispatcher loop did not complete cleanly");
        }
    }
}

pub(super) fn start(service: &TaskService) {
    let handle = start_with_shutdown(service, service.dispatcher_shutdown());
    service.store_dispatcher_handle(handle);
}

/// Cancellation-aware dispatcher start. Returns `None` when this replica has
/// already started its dispatcher.
pub(super) fn start_with_shutdown(
    service: &TaskService,
    shutdown: CancellationToken,
) -> Option<DispatcherHandle> {
    if !service.dispatcher_started() {
        return None;
    }

    let service = service.clone();
    let completion = tokio::spawn(async move {
        // Startup path: run maintenance once so any exhausted items or
        // expired attempts left by the previous process converge before
        // the first claim, then drain the available queue.
        run_maintenance(&service).await;
        if !shutdown.is_cancelled() {
            dispatch_available(&service, &shutdown).await;
        }

        let mut recovery = interval(RECOVERY_INTERVAL);
        recovery.set_missed_tick_behavior(MissedTickBehavior::Delay);
        // The first interval tick completes immediately. Consume it so the
        // first periodic recovery runs 30 seconds after startup, not
        // milliseconds after the explicit startup maintenance above.
        recovery.tick().await;

        loop {
            tokio::select! {
                _ = shutdown.cancelled() => break,
                _ = service.dispatch_notify().notified() => {
                    // Notification-driven wake: only the fast claim. The
                    // maintenance UPDATE/RETURNING work is reserved for the
                    // periodic recovery tick so spammed submit/retry/finish
                    // notifications avoid the exhausted/expired work.
                    dispatch_available(&service, &shutdown).await;
                }
                _ = recovery.tick() => {
                    // Recovery tick: maintenance must converge exhausted
                    // items and expired attempts even when no row is
                    // claimable, then drain any newly eligible work.
                    // Sequential so the fast dispatch sees freshly
                    // converged state. Failure is logged, never propagated.
                    run_maintenance(&service).await;
                    if shutdown.is_cancelled() {
                        break;
                    }
                    dispatch_available(&service, &shutdown).await;
                }
            }
        }
    });
    Some(DispatcherHandle { completion })
}

/// One claim source the dispatch loop can drain.
///
/// The production implementation is [`TaskService`]; tests use a fake so the
/// cancellation boundary is exercised without a database.
trait TaskClaimSource {
    type Item;

    fn available_worker_slots(&self) -> usize;
    fn claim_capacity(&self) -> i64;
    async fn claim_items(&self, capacity: i64) -> Result<Vec<Self::Item>>;
    fn dispatch_item(&self, item: Self::Item) -> bool;
    fn log_no_claimable(&self);
}

impl TaskClaimSource for TaskService {
    type Item = crate::db::ClaimedItem;

    fn available_worker_slots(&self) -> usize {
        self.available_worker_slots()
    }

    // The durable parent-task capacity is global: every replica passes the same
    // configured `scheduler.max_concurrency`, and the database admits at most
    // that many parents across all replicas. The local semaphore only bounds
    // this replica's workers.
    fn claim_capacity(&self) -> i64 {
        i64::try_from(self.worker_capacity()).unwrap_or(i64::MAX)
    }

    async fn claim_items(&self, capacity: i64) -> Result<Vec<Self::Item>> {
        self.db().claim_items_fast(capacity).await
    }

    fn dispatch_item(&self, item: Self::Item) -> bool {
        log_claim(&item);
        let Ok(permit) = self.worker_slots().try_acquire_owned() else {
            return false;
        };
        self.spawn_item(item, permit);
        true
    }

    fn log_no_claimable(&self) {
        log_no_claimable_item(self.worker_capacity(), self.available_worker_slots());
    }
}

async fn dispatch_available(service: &TaskService, shutdown: &CancellationToken) {
    let claimed_total = dispatch_claims(service, shutdown).await;
    log_claims_dispatched(
        claimed_total,
        service.worker_capacity(),
        service.available_worker_slots(),
    );
}

/// Claim until the queue drains, the replica runs out of slots, or shutdown is
/// observed.
///
/// The cancellation check sits before each claim, so a claim already issued when
/// the token fires is still dispatched as owned work: the durable lease exists
/// from that point on, and abandoning it locally would hide work the database
/// already admitted. No further claim is issued after the token is observed.
async fn dispatch_claims<S: TaskClaimSource>(source: &S, shutdown: &CancellationToken) -> usize {
    let mut claimed_total = 0usize;
    loop {
        if shutdown.is_cancelled() {
            break;
        }
        if source.available_worker_slots() == 0 {
            break;
        }
        // Hot path: only the admission claim statement runs (plus the
        // admission lock that serializes it across replicas). Maintenance
        // lives on the recovery tick so notification-driven wakes (submit,
        // retry, finish, worker release) avoid the exhausted/expired
        // UPDATE CTEs that used to run on every wake.
        let items = match source.claim_items(source.claim_capacity()).await {
            Ok(items) => items,
            Err(error) => {
                tracing::warn!(%error, "failed to claim context69 task items");
                break;
            }
        };
        if items.is_empty() {
            source.log_no_claimable();
            break;
        }
        for item in items {
            if !source.dispatch_item(item) {
                break;
            }
            claimed_total += 1;
        }
    }
    claimed_total
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
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use std::time::Duration;

    use anyhow::Result;
    use futures::{StreamExt, stream::FuturesUnordered};
    use tokio::{
        sync::{Notify, Semaphore},
        time::timeout,
    };
    use tokio_util::sync::CancellationToken;

    use super::dispatch_claims;

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

    #[derive(Default)]
    struct FakeClaimSource {
        batches: Mutex<Vec<Vec<usize>>>,
        claim_calls: AtomicUsize,
        dispatched: Mutex<Vec<usize>>,
        cancel_on_claim: Option<CancellationToken>,
        slots: usize,
    }

    impl FakeClaimSource {
        fn with_batches(batches: Vec<Vec<usize>>) -> Self {
            Self {
                batches: Mutex::new(batches),
                slots: 4,
                ..Self::default()
            }
        }
    }

    impl super::TaskClaimSource for FakeClaimSource {
        type Item = usize;

        fn available_worker_slots(&self) -> usize {
            self.slots
        }

        fn claim_capacity(&self) -> i64 {
            4
        }

        async fn claim_items(&self, _capacity: i64) -> Result<Vec<Self::Item>> {
            self.claim_calls.fetch_add(1, Ordering::SeqCst);
            if let Some(shutdown) = self.cancel_on_claim.as_ref() {
                shutdown.cancel();
            }
            let mut batches = self.batches.lock().expect("fake batches");
            if batches.is_empty() {
                Ok(Vec::new())
            } else {
                Ok(batches.remove(0))
            }
        }

        fn dispatch_item(&self, item: Self::Item) -> bool {
            self.dispatched.lock().expect("fake dispatched").push(item);
            true
        }

        fn log_no_claimable(&self) {}
    }

    #[tokio::test]
    async fn cancellation_before_claim_issues_no_claim() {
        let shutdown = CancellationToken::new();
        shutdown.cancel();
        let source = FakeClaimSource::with_batches(vec![vec![1]]);

        let claimed = dispatch_claims(&source, &shutdown).await;

        assert_eq!(claimed, 0);
        assert_eq!(source.claim_calls.load(Ordering::SeqCst), 0);
        assert!(
            source
                .dispatched
                .lock()
                .expect("fake dispatched")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn cancellation_during_claim_dispatches_the_owned_claim_then_stops() {
        let shutdown = CancellationToken::new();
        let source = FakeClaimSource {
            batches: Mutex::new(vec![vec![1, 2], vec![3]]),
            cancel_on_claim: Some(shutdown.clone()),
            slots: 4,
            ..FakeClaimSource::default()
        };

        let claimed = dispatch_claims(&source, &shutdown).await;

        assert_eq!(claimed, 2);
        assert_eq!(source.claim_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            *source.dispatched.lock().expect("fake dispatched"),
            vec![1, 2]
        );
    }

    #[tokio::test]
    async fn normal_dispatch_drains_every_batch_until_empty() {
        let shutdown = CancellationToken::new();
        let source = FakeClaimSource::with_batches(vec![vec![1, 2], vec![3], vec![]]);

        let claimed = dispatch_claims(&source, &shutdown).await;

        assert_eq!(claimed, 3);
        assert_eq!(source.claim_calls.load(Ordering::SeqCst), 3);
        assert_eq!(
            *source.dispatched.lock().expect("fake dispatched"),
            vec![1, 2, 3]
        );
    }
}
