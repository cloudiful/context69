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

/// Minimum delay for a due known Docling probe, so a past-due or
/// full-capacity queue recomputes instead of spinning.
const MIN_PROBE_DELAY: Duration = Duration::from_secs(1);

fn probe_delay(remaining_ms: Option<i64>) -> Duration {
    match remaining_ms {
        Some(remaining_ms) => {
            let millis = remaining_ms.max(MIN_PROBE_DELAY.as_millis() as i64) as u64;
            Duration::from_millis(millis)
        }
        None => RECOVERY_INTERVAL,
    }
}

async fn next_probe_wait(service: &TaskService) -> tokio::time::Sleep {
    let delay = match service.db().next_docling_probe_delay().await {
        Ok(remaining_ms) => probe_delay(remaining_ms),
        Err(error) => {
            tracing::warn!(%error, "failed to read the next Docling probe delay");
            RECOVERY_INTERVAL
        }
    };
    tokio::time::sleep(delay)
}

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
        // The execution latch is the fail-closed startup gate: while abandoned
        // legacy work or an unusable library schema is present, no dispatcher,
        // no initial maintenance and no claim runs. The latch logs the blocker
        // and its remediation once, and never re-evaluates, so a handover
        // performed under a running process does not open the engine.
        if !service.execution_readiness().await.is_ready() {
            return;
        }
        // Startup path: run maintenance once so any exhausted items or
        // expired attempts left by the previous process converge before
        // the first claim, then drain the available queue.
        run_maintenance(&service).await;
        if !shutdown.is_cancelled() {
            dispatch_available(&service, &shutdown).await;
        }
        let mut probe = Box::pin(next_probe_wait(&service).await);

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
                    probe = Box::pin(next_probe_wait(&service).await);
                }
                _ = &mut probe => {
                    // Known Docling job due: the same claim loop takes the
                    // parked item and one probe runs inside its segment. The
                    // delay is recomputed from the database clock after every
                    // dispatch, so a past-due job never busy-spins.
                    dispatch_available(&service, &shutdown).await;
                    probe = Box::pin(next_probe_wait(&service).await);
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
                    probe = Box::pin(next_probe_wait(&service).await);
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

    async fn claim_items(&self) -> Result<(Vec<Self::Item>, usize)>;
    fn dispatch_item(&self, item: Self::Item);
    fn log_no_claimable(&self);
}

impl TaskClaimSource for TaskService {
    type Item = crate::db::ClaimedItem;

    // Execution capacity is the library queue policy: the claim asks for work
    // and the library grants nothing while the durable cap is full, so this
    // loop never carries a second capacity estimate.
    async fn claim_items(&self) -> Result<(Vec<Self::Item>, usize)> {
        let outcome = self.db().claim_items_fast_dispatch().await?;
        Ok((outcome.claimed, outcome.unclassified))
    }

    fn dispatch_item(&self, item: Self::Item) {
        log_claim(&item);
        self.spawn_item(item);
    }

    fn log_no_claimable(&self) {
        log_no_claimable_item();
    }
}

async fn dispatch_available(service: &TaskService, shutdown: &CancellationToken) {
    let claimed_total = dispatch_claims(service, shutdown).await;
    log_claims_dispatched(claimed_total);
}

/// Claim until the library grants nothing, or shutdown is observed.
///
/// The cancellation check sits before each claim, so a claim already issued when
/// the token fires is still dispatched as owned work: the durable lease exists
/// from that point on, and abandoning it locally would hide work the database
/// already admitted. No further claim is issued after the token is observed.
/// A full queue policy is simply an empty grant, so the loop needs no local
/// capacity estimate.
async fn dispatch_claims<S: TaskClaimSource>(source: &S, shutdown: &CancellationToken) -> usize {
    let mut claimed_total = 0usize;
    let mut reported_unclassified = false;
    loop {
        if shutdown.is_cancelled() {
            break;
        }
        // Hot path: only the claim statement and the bounded expired-segment
        // recovery inside it run. Maintenance lives on the recovery tick so
        // notification-driven wakes (submit, retry, finish, worker release)
        // avoid the exhausted/expired UPDATE CTEs that used to run on every
        // wake.
        let (items, unclassified) = match source.claim_items().await {
            Ok(outcome) => outcome,
            Err(error) => {
                tracing::warn!(%error, "failed to claim context69 task items");
                break;
            }
        };
        if unclassified > 0 && !reported_unclassified {
            reported_unclassified = true;
            tracing::warn!(
                unclassified,
                "eligible task heads have no valid admission class; they are not claimed and require the explicit task_execution_cutover reclassify-admission command"
            );
        }
        if items.is_empty() {
            source.log_no_claimable();
            break;
        }
        for item in items {
            source.dispatch_item(item);
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
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use std::time::Duration;

    use anyhow::Result;
    use tokio::{sync::Notify, time::timeout};
    use tokio_util::sync::CancellationToken;

    use super::{RECOVERY_INTERVAL, dispatch_claims, probe_delay};

    #[test]
    fn a_due_probe_waits_at_least_one_second_and_absent_work_defers_to_recovery() {
        assert_eq!(
            probe_delay(Some(0)),
            std::time::Duration::from_secs(1),
            "a past-due job must not busy-spin"
        );
        assert_eq!(
            probe_delay(Some(-50)),
            std::time::Duration::from_secs(1),
            "a negative remaining delay is clamped to the floor"
        );
        assert_eq!(
            probe_delay(Some(2_500)),
            std::time::Duration::from_millis(2_500)
        );
        assert_eq!(
            probe_delay(None),
            RECOVERY_INTERVAL,
            "no known job keeps the ordinary recovery cadence"
        );
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
    }

    impl FakeClaimSource {
        fn with_batches(batches: Vec<Vec<usize>>) -> Self {
            Self {
                batches: Mutex::new(batches),
                ..Self::default()
            }
        }
    }

    impl super::TaskClaimSource for FakeClaimSource {
        type Item = usize;

        async fn claim_items(&self) -> Result<(Vec<Self::Item>, usize)> {
            self.claim_calls.fetch_add(1, Ordering::SeqCst);
            if let Some(shutdown) = self.cancel_on_claim.as_ref() {
                shutdown.cancel();
            }
            let mut batches = self.batches.lock().expect("fake batches");
            if batches.is_empty() {
                Ok((Vec::new(), 0))
            } else {
                Ok((batches.remove(0), 0))
            }
        }

        fn dispatch_item(&self, item: Self::Item) {
            self.dispatched.lock().expect("fake dispatched").push(item);
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
