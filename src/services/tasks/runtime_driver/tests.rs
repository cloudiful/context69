//! Tests for the claim-run driver.
//!
//! One pure case pins the stage/kind vocabulary, and two cases pin the ownership
//! of the work handle: the real `ClaimRun` is aborted while its work is pending
//! (`an_outer_cancellation_drops_the_retained_work`), and the real driver future
//! is cancelled on the production path with its spawned work parked at an
//! injected gate (`outer_driver_cancellation`).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use super::{ClaimRun, parse_kind};
use crate::domain_errors::{DomainError, find_domain_error};

#[test]
fn unknown_task_kind_is_typed_invalid_argument() {
    let Err(error) = parse_kind("bogus") else {
        panic!("expected unknown task kind to fail");
    };
    assert!(matches!(
        find_domain_error(&error),
        Some(DomainError::InvalidArgument(_))
    ));
    assert_eq!(error.to_string(), "unsupported task kind bogus");
}

/// The work of an aborted driver is dropped with it.
///
/// This drives the real `ClaimRun` and the real await of the driver: the work is
/// a task that reports that it started and then parks until the case releases
/// it. Aborting the outer driver while that await is pending must drop the
/// started work instead of detaching it, so the release cannot make it continue.
#[tokio::test]
async fn an_outer_cancellation_drops_the_retained_work() {
    let continued = Arc::new(AtomicBool::new(false));
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let flag = Arc::clone(&continued);
    let driver = tokio::spawn(async move {
        let work = tokio::spawn(async move {
            let _ = started_tx.send(());
            let _ = release_rx.await;
            flag.store(true, Ordering::SeqCst);
            Ok(super::ProcessResult::Succeeded(None))
        });
        let work_abort = work.abort_handle();
        let mut run = ClaimRun {
            work_abort,
            work: Some(work),
            heartbeat: super::super::runtime::HeartbeatGuard::inert(),
        };
        let _ = run.work().await;
    });
    started_rx.await.expect("the work reports that it started");
    tokio::task::yield_now().await;

    driver.abort();
    let _ = driver.await;
    let _ = release_tx.send(());
    tokio::time::sleep(Duration::from_millis(50)).await;

    assert!(
        !continued.load(Ordering::SeqCst),
        "the work of a cancelled driver must be dropped instead of continuing"
    );
}

/// The gate the driver's pipeline body is held at.
///
/// The production build runs the gate as an async no-op; a case installs one so
/// the real driver future is cancelled while its spawned work is genuinely
/// pending. It records that the body reached the gate, whether it was released
/// into the pipeline, and whether the parked body was dropped (which is what the
/// cancellation of the driver does).
pub(crate) mod work_gate {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, OnceLock};

    pub(crate) struct Gate {
        release: tokio::sync::Notify,
        reached: AtomicBool,
        passed: AtomicBool,
        dropped: AtomicBool,
    }

    impl Gate {
        pub(crate) fn new() -> Arc<Self> {
            Arc::new(Self {
                release: tokio::sync::Notify::new(),
                reached: AtomicBool::new(false),
                passed: AtomicBool::new(false),
                dropped: AtomicBool::new(false),
            })
        }

        /// Lets a parked body continue into the pipeline.
        pub(crate) fn release(&self) {
            self.release.notify_one();
        }

        pub(crate) fn reached(&self) -> bool {
            self.reached.load(Ordering::SeqCst)
        }

        pub(crate) fn passed(&self) -> bool {
            self.passed.load(Ordering::SeqCst)
        }

        pub(crate) fn dropped(&self) -> bool {
            self.dropped.load(Ordering::SeqCst)
        }
    }

    /// Marks a parked body that was dropped instead of being released.
    struct Parked(Arc<Gate>);

    impl Drop for Parked {
        fn drop(&mut self) {
            if !self.0.passed.load(Ordering::SeqCst) {
                self.0.dropped.store(true, Ordering::SeqCst);
            }
        }
    }

    static GATE: OnceLock<Mutex<Option<Arc<Gate>>>> = OnceLock::new();

    fn cell() -> &'static Mutex<Option<Arc<Gate>>> {
        GATE.get_or_init(|| Mutex::new(None))
    }

    pub(crate) fn install(gate: &Arc<Gate>) {
        *cell().lock().expect("gate lock") = Some(Arc::clone(gate));
    }

    pub(crate) fn clear() {
        *cell().lock().expect("gate lock") = None;
    }

    pub(crate) async fn wait() {
        let gate = cell().lock().expect("gate lock").clone();
        let Some(gate) = gate else {
            return;
        };
        let _parked = Parked(Arc::clone(&gate));
        gate.reached.store(true, Ordering::SeqCst);
        gate.release.notified().await;
        gate.passed.store(true, Ordering::SeqCst);
    }
}

/// The outer cancellation of the real driver, against the real database.
///
/// The production `drive_claimed_item` future runs with the pipeline body parked
/// at the installed gate. Cancelling the driver must drop that parked work
/// instead of letting it continue: the case releases the gate afterwards and
/// proves the body never reached the pipeline, the renewal stopped with the
/// driver, and the item kept nothing the released body would have written.
mod outer_driver_cancellation {
    use std::time::Duration;

    use serde_json::json;
    use uuid::Uuid;

    use crate::db::{CreateTaskSubmissionRequest, Database};
    use crate::services::tasks::TASK_CASE_LOCK;
    use crate::services::tasks::item_pipeline_tests::support::build_service;

    use super::super::{drive_claimed_item, parse_kind};
    use super::work_gate::{self, Gate};

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
            format!("outer-cancel-{}", Uuid::new_v4()),
            "Outer Cancel Test",
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
            group_path: Some("test/outer-cancel"),
            source_key: None,
            payloads: &[json!({"external_id": "a"})],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: &format!("outer-cancel-{}", Uuid::new_v4()),
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

    async fn status(db: &Database, item_id: Uuid) -> String {
        sqlx::query_file_scalar!("tests/sql/task_execution/read_item_status.sql", item_id)
            .fetch_one(db.pool())
            .await
            .expect("read the status")
    }

    async fn lease_expires_at(
        db: &Database,
        attempt_id: Uuid,
    ) -> Option<chrono::DateTime<chrono::Utc>> {
        sqlx::query_file_scalar!(
            "tests/sql/task_execution/read_library_attempt_lease.sql",
            attempt_id
        )
        .fetch_one(db.pool())
        .await
        .expect("read the library attempt lease")
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

    async fn wait_for<F: Fn() -> bool>(observed: F) -> bool {
        for _ in 0..400 {
            if observed() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        false
    }

    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn an_outer_driver_cancellation_drops_the_running_work() {
        let db = connect().await;
        let _guard = TASK_CASE_LOCK.lock().await;
        let (user_id, task_id, item) = seed_claim(&db).await;
        let item_id = item.id;
        let execution = item
            .execution_context()
            .expect("the claimed item carries its captured execution");
        let attempt_id = execution.attempt_id();
        let service = build_service(&db).await.0;
        let task = service.task(item.task_id).await.expect("read the task");
        let kind = parse_kind(&item.kind).expect("the item kind is known");
        let before_payload = payload(&db, item_id).await;
        let before_status = status(&db, item_id).await;

        // The renewal runs with a shortened timing so that "the heartbeat is gone
        // with the driver" is observable inside one case, and the gate holds the
        // spawned pipeline body before it does any work.
        super::super::super::runtime::test_timing::install(
            Duration::from_millis(10),
            Duration::from_millis(50),
        );
        let gate = Gate::new();
        work_gate::install(&gate);

        let driver = tokio::spawn({
            let service = service.clone();
            async move { drive_claimed_item(&service, item, task, kind).await }
        });
        assert!(
            wait_for(|| gate.reached()).await,
            "the driver must have spawned the work and parked it at the gate"
        );

        // At least one renewal must have landed, so the stillness check below is
        // not satisfied by a renewal that never ran.
        let renewed = lease_expires_at(&db, attempt_id).await;
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(
            lease_expires_at(&db, attempt_id).await > renewed,
            "the heartbeat must be renewing the attempt lease before the cancellation"
        );

        // Cancel the outer driver exactly as a shutdown or a dropped future does.
        driver.abort();
        let _ = driver.await;
        gate.release();
        tokio::time::sleep(Duration::from_millis(150)).await;
        let settled = lease_expires_at(&db, attempt_id).await;
        tokio::time::sleep(Duration::from_millis(250)).await;

        assert!(
            !gate.passed(),
            "the parked pipeline body must not continue after the driver was cancelled"
        );
        assert!(
            gate.dropped(),
            "the parked pipeline body must be dropped with the cancelled driver"
        );
        assert_eq!(
            lease_expires_at(&db, attempt_id).await,
            settled,
            "no heartbeat may renew the lease after the driver is cancelled"
        );
        assert_eq!(
            payload(&db, item_id).await,
            before_payload,
            "the cancelled run must not write the item payload"
        );
        assert_eq!(
            status(&db, item_id).await,
            before_status,
            "the cancelled run must not transition the item"
        );

        work_gate::clear();
        super::super::super::runtime::test_timing::clear();
        cleanup(&db, task_id, user_id).await;
    }
}
