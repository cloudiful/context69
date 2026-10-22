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
///
/// The production values are the constants above. Only a test of the runtime
/// can shorten them, so the abort path can be observed inside one case without
/// introducing a runtime configuration knob or a public back door.
fn renewal_timing() -> (Duration, Duration) {
    #[cfg(test)]
    if let Some(timing) = test_timing::current() {
        return timing;
    }
    (HEARTBEAT_INTERVAL, HEARTBEAT_RENEW_TIMEOUT)
}

#[cfg(test)]
pub(crate) mod test_timing {
    use std::sync::{Mutex, OnceLock};
    use std::time::Duration;

    static TIMING: OnceLock<Mutex<Option<(Duration, Duration)>>> = OnceLock::new();

    fn cell() -> &'static Mutex<Option<(Duration, Duration)>> {
        TIMING.get_or_init(|| Mutex::new(None))
    }

    /// Shortens the renewal timing for one case.
    pub(crate) fn install(interval: Duration, timeout: Duration) {
        *cell().lock().expect("timing lock") = Some((interval, timeout));
    }

    /// Restores the production timing.
    pub(crate) fn clear() {
        *cell().lock().expect("timing lock") = None;
    }

    pub(super) fn current() -> Option<(Duration, Duration)> {
        *cell().lock().expect("timing lock")
    }
}

/// Aborts the wrapped heartbeat task when dropped so that early `?` returns
/// from `run_item` cannot leave an orphaned heartbeat renewing a lease forever.
pub(super) struct HeartbeatGuard(JoinHandle<()>);

impl HeartbeatGuard {
    pub(super) fn abort(&self) {
        self.0.abort();
    }
}

#[cfg(test)]
impl HeartbeatGuard {
    /// A guard over an inert task, for a case that exercises the ownership of
    /// the work handle without a real renewal running beside it.
    pub(super) fn inert() -> Self {
        Self(tokio::spawn(std::future::pending::<()>()))
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
async fn heartbeat_loop<F, Fut>(mut tick: F, interval_duration: Duration, abort_work: AbortHandle)
where
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
                abort_work.abort();
                break;
            }
            Ok(Err(_)) | Err(_) => {
                consecutive_errors += 1;
                if consecutive_errors >= HEARTBEAT_MAX_CONSECUTIVE_ERRORS {
                    // Database unavailable: stop renewing so the lease expires
                    // and the item becomes recoverable by another worker. The
                    // work is dropped instead of running on a lease that is
                    // about to expire.
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
    HeartbeatGuard(tokio::spawn(heartbeat_loop(
        move || {
            let service = service.clone();
            let execution = execution.clone();
            async move { service.db().heartbeat_task_item(&execution).await }
        },
        interval,
        abort_work,
    )))
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    };

    use tokio::task::AbortHandle;

    use super::{HEARTBEAT_MAX_CONSECUTIVE_ERRORS, heartbeat_loop};

    fn test_interval() -> std::time::Duration {
        std::time::Duration::from_millis(1)
    }

    /// A handle of a task that never runs, so the abort of the loop is inert.
    fn inert_abort_handle() -> AbortHandle {
        tokio::spawn(std::future::pending::<()>()).abort_handle()
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
            inert_abort_handle(),
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
            inert_abort_handle(),
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
            inert_abort_handle(),
        )
        .await;
        assert_eq!(ticks.load(Ordering::SeqCst), 1);
    }

    /// The runtime abort path, against the real database and the real heartbeat.
    ///
    /// The work is a real task that ignores any cooperative signal: it signals
    /// that it started and then sleeps far past the renewal budget. The
    /// authoritative library attempt is then blocked or expired, the heartbeat
    /// runs with the shortened timing, and the started work must be dropped with
    /// no late write reaching the item.
    mod runtime_abort {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::time::Duration;

        use serde_json::json;
        use uuid::Uuid;

        use crate::db::{CreateTaskSubmissionRequest, Database};
        use crate::services::tasks::TaskService;

        static CASE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

        /// A minimal service: the heartbeat only needs the database handle.
        async fn build_service(db: &Database) -> TaskService {
            crate::services::tasks::item_pipeline_tests::support::build_service(db)
                .await
                .0
        }

        async fn connect() -> Database {
            let url = context69_test_support::accessor::database_url();
            let db = Database::connect(&url)
                .await
                .expect("connect test database");
            context69_db_schema::bootstrap_library_schema(db.pool())
                .await
                .expect("bootstrap the library schema");
            db
        }

        async fn seed_claim(db: &Database) -> (i64, Uuid, crate::db::ClaimedItem) {
            let user_id: i64 = sqlx::query_file_scalar!(
                "tests/sql/task_execution/seed_user.sql",
                format!("runtime-abort-{}", Uuid::new_v4()),
                "Runtime Abort Test",
                "unused"
            )
            .fetch_one(db.pool())
            .await
            .expect("seed user");
            let task_id = Uuid::new_v4();
            db.create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
                task_id,
                user_id,
                group_id: None,
                kind: "text_batch",
                group_path: Some("test/runtime-abort"),
                source_key: None,
                payloads: &[json!({"external_id": "a"})],
                input_storage_object_ids: None,
                idempotency_key: None,
                request_hash: &format!("runtime-abort-{}", Uuid::new_v4()),
            })
            .await
            .expect("create task");
            let item = db
                .claim_items_fast(10)
                .await
                .expect("claim the item")
                .into_iter()
                .find(|item| item.task_id == task_id)
                .expect("the item is claimed");
            (user_id, task_id, item)
        }

        async fn payload(db: &Database, item_id: Uuid) -> serde_json::Value {
            sqlx::query_file_scalar!("tests/sql/task_execution/read_item_payload.sql", item_id)
                .fetch_one(db.pool())
                .await
                .expect("read the payload")
        }

        async fn cleanup(db: &Database, task_id: Uuid, user_id: i64) {
            sqlx::query_file!(
                "tests/sql/task_execution/delete_task_items_by_task.sql",
                task_id
            )
            .execute(db.pool())
            .await
            .expect("delete the task items");
            sqlx::query_file!("tests/sql/task_execution/delete_task_by_id.sql", task_id)
                .execute(db.pool())
                .await
                .expect("delete the task");
            sqlx::query_file!("tests/sql/task_execution/delete_user.sql", user_id)
                .execute(db.pool())
                .await
                .expect("delete user");
        }

        /// Starts the work that must be dropped and returns its handle plus the
        /// flag it sets when it actually starts.
        fn start_uncooperative_work(
            service: TaskService,
            item: crate::db::ClaimedItem,
            started: Arc<AtomicBool>,
        ) -> tokio::task::JoinHandle<()> {
            tokio::spawn(async move {
                started.store(true, Ordering::SeqCst);
                // No cooperative cancellation point: the body only ends when the
                // task is dropped. A late write is attempted afterwards, so an
                // abort that did not happen would still be visible.
                tokio::time::sleep(Duration::from_secs(30)).await;
                let payload = json!({"late_write": true});
                let _ = service
                    .db()
                    .set_task_item_payload(item.id, item.lease_token, &payload)
                    .await;
            })
        }

        async fn wait_started(started: &AtomicBool) {
            for _ in 0..200 {
                if started.load(Ordering::SeqCst) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            panic!("the work never started");
        }

        #[tokio::test]
        #[ignore = "requires configured reusable PostgreSQL"]
        async fn a_row_locked_renewal_times_out_and_drops_the_started_work() {
            let db = connect().await;
            let _guard = CASE_LOCK.lock().await;
            let (user_id, task_id, item) = seed_claim(&db).await;
            let execution = item
                .execution_context()
                .expect("the claimed item carries its captured execution");
            let before = payload(&db, item.id).await;

            // A second connection holds the authoritative attempt row, so the
            // renewal blocks until the heartbeat timeout fires.
            let mut blocker = db.pool().acquire().await.expect("blocking connection");
            sqlx::query_file!("tests/sql/task_execution/begin_transaction.sql")
                .execute(&mut *blocker)
                .await
                .expect("begin blocker");
            sqlx::query_file_scalar!(
                "tests/sql/task_execution/lock_library_attempt_by_id.sql",
                execution.attempt_id()
            )
            .fetch_optional(&mut *blocker)
            .await
            .expect("lock the library attempt");

            super::super::test_timing::install(
                Duration::from_millis(10),
                Duration::from_millis(50),
            );
            let started = Arc::new(AtomicBool::new(false));
            let service = build_service(&db).await;
            let work = start_uncooperative_work(service.clone(), item.clone(), started.clone());
            wait_started(&started).await;

            let heartbeat = super::super::spawn_item_heartbeat(
                service.clone(),
                execution.clone(),
                work.abort_handle(),
            );
            for _ in 0..200 {
                if work.is_finished() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            assert!(
                work.is_finished(),
                "the started work must be dropped once the renewal budget is spent"
            );
            heartbeat.abort();
            super::super::test_timing::clear();
            sqlx::query_file!("tests/sql/task_execution/rollback_transaction.sql")
                .execute(&mut *blocker)
                .await
                .expect("release the blocker");
            drop(blocker);

            assert_eq!(
                payload(&db, item.id).await,
                before,
                "the dropped work must not write anything afterwards"
            );
            cleanup(&db, task_id, user_id).await;
        }

        #[tokio::test]
        #[ignore = "requires configured reusable PostgreSQL"]
        async fn a_lost_lease_drops_the_started_work_without_a_late_write() {
            let db = connect().await;
            let _guard = CASE_LOCK.lock().await;
            let (user_id, task_id, item) = seed_claim(&db).await;
            let execution = item
                .execution_context()
                .expect("the claimed item carries its captured execution");
            let before = payload(&db, item.id).await;

            super::super::test_timing::install(
                Duration::from_millis(10),
                Duration::from_millis(50),
            );
            let started = Arc::new(AtomicBool::new(false));
            let service = build_service(&db).await;
            let work = start_uncooperative_work(service.clone(), item.clone(), started.clone());
            wait_started(&started).await;

            // The authoritative attempt expires, exactly as a crashed worker's
            // lease does; the domain mirror is left alone so the refusal comes
            // from the library, not from a backdated row.
            sqlx::query_file!(
                "tests/sql/task_execution/expire_library_attempt_by_id.sql",
                execution.attempt_id()
            )
            .execute(db.pool())
            .await
            .expect("expire the library attempt");

            let heartbeat = super::super::spawn_item_heartbeat(
                service.clone(),
                execution.clone(),
                work.abort_handle(),
            );
            for _ in 0..200 {
                if work.is_finished() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            assert!(
                work.is_finished(),
                "a lost lease drops the started work instead of only stopping the renewal"
            );
            heartbeat.abort();
            super::super::test_timing::clear();

            assert_eq!(
                payload(&db, item.id).await,
                before,
                "the dropped work must not write anything afterwards"
            );
            cleanup(&db, task_id, user_id).await;
        }
    }
}
