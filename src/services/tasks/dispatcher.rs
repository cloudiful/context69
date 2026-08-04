use std::time::Duration;

use tokio::time::interval;

use super::TaskService;

const RECOVERY_INTERVAL: Duration = Duration::from_secs(30);

pub(super) fn start(service: &TaskService) {
    if !service.dispatcher_started() {
        return;
    }

    let service = service.clone();
    tokio::spawn(async move {
        let mut recovery = interval(RECOVERY_INTERVAL);
        dispatch_available(&service).await;
        loop {
            tokio::select! {
                _ = service.dispatch_notify().notified() => {}
                _ = recovery.tick() => {}
            }
            dispatch_available(&service).await;
        }
    });
}

async fn dispatch_available(service: &TaskService) {
    let pending_count = match service.db().pending_task_count().await {
        Ok(count) => count,
        Err(error) => {
            tracing::warn!(%error, "failed to count pending context69 tasks");
            return;
        }
    };
    let available_slots = service.available_worker_slots();
    if available_slots == 0 {
        tracing::info!(
            target: "task_dispatch",
            pending_count,
            inflight_count = service.worker_capacity(),
            available_slots,
            "task dispatcher state"
        );
        return;
    }

    let limit = dispatch_limit(available_slots);
    let task_ids = match service.db().pending_task_ids(limit).await {
        Ok(task_ids) => task_ids,
        Err(error) => {
            tracing::warn!(%error, "failed to load pending context69 tasks");
            return;
        }
    };

    for task_id in task_ids {
        let Ok(permit) = service.worker_slots().try_acquire_owned() else {
            break;
        };
        service.spawn(task_id, permit);
    }

    let available_slots = service.available_worker_slots();
    tracing::info!(
        target: "task_dispatch",
        pending_count,
        inflight_count = service.worker_capacity().saturating_sub(available_slots),
        available_slots,
        "task dispatcher state"
    );
}

fn dispatch_limit(available_slots: usize) -> i64 {
    i64::try_from(available_slots).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use futures::{StreamExt, stream::FuturesUnordered};
    use tokio::{
        sync::{Notify, Semaphore},
        time::timeout,
    };

    use super::dispatch_limit;

    #[test]
    fn dispatcher_query_limit_matches_available_slots() {
        assert_eq!(dispatch_limit(1), 1);
        assert_eq!(dispatch_limit(4), 4);
        assert_eq!(dispatch_limit(usize::MAX), i64::MAX);
    }

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
