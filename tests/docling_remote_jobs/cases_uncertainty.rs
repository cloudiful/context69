//! Fail-closed Docling dispatch intent and unknown-outcome cases (issue 1172 P5A).
//!
//! The durable intent is persisted before a dispatch is prepared, a known id
//! alone moves it to the P4 poll path, and every unknown outcome parks the item
//! without a due time and without a second POST.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use chrono::{Duration, Utc};
use cloudiful_tasks::TaskId;
use context69::db::{
    CreateTaskSubmissionRequest, Database, DoclingRiskAck, DoclingSubmissionFence,
    DoclingSubmissionIntent, DoclingSubmissionOutcome, ParkSubmissionUncertainRequest,
    StoredDoclingSettings,
};
use context69_contracts::tasks::TaskDoclingSubmissionState;
use context69_contracts::{
    TaskBatchGetRequest, TaskBatchRetryOutcome, TaskBatchRetryReason, TaskBatchRetryRequest,
    TaskListQuery, TaskListView,
};
use context69_test_support::execution_cleanup::{
    FixtureExecutionScope, quiesce_fixture_executions,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::qdrant_points_mock::QdrantPointsMock;
use crate::source_rerun_support::{
    self, FakeEmbedding, build_service, seed_group, seed_pdf_source_file, seed_succeeded_source,
};
use crate::support::{LOCK, claim_item, cleanup, connect, crash_holder, seed_task, seed_user};

const UNCERTAIN_REASON: &str =
    "the Docling dispatch outcome is unknown; acknowledgement is required";

type SubmissionState = (
    String,
    Option<String>,
    bool,
    Option<Uuid>,
    Option<i64>,
    serde_json::Value,
    Option<String>,
);

async fn submission_state(db: &Database, job_id: Uuid) -> SubmissionState {
    let row = sqlx::query_file!(
        "tests/sql/docling_remote_jobs/read_submission_state.sql",
        job_id
    )
    .fetch_one(db.pool())
    .await
    .expect("read the submission state");
    (
        row.status,
        row.remote_task_id,
        row.dispatch_attempted,
        row.submission_execution_task_id,
        row.submission_generation,
        row.submission_identity,
        row.last_error,
    )
}

async fn failure_budget(db: &Database, item_id: Uuid) -> i32 {
    sqlx::query_file_scalar!(
        "tests/sql/task_execution/read_execution_failure_budget.sql",
        item_id
    )
    .fetch_one(db.pool())
    .await
    .expect("read the failure budget")
}

fn fence_of(
    task_id: Uuid,
    item: &context69::db::ClaimedItem,
) -> (DoclingSubmissionFence, context69::db::ExecutionContext) {
    let execution = item
        .execution_context()
        .expect("the claimed item carries its captured execution");
    (
        DoclingSubmissionFence {
            task_id,
            item_id: item.id,
            lease_token: item.lease_token,
            execution_task_id: execution.task_id(),
            generation: execution.generation,
        },
        execution,
    )
}

/// The intent is durable, one per item, and only a confirmed id leaves the
/// unknown state; a preparation failure closes it before any dispatch.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_durable_intent_stays_fail_closed_until_a_known_id_is_confirmed() {
    let db = connect().await;
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id).await;
    let claimed = claim_item(&db, item_id).await;
    let (fence, execution) = fence_of(task_id, &claimed);
    let identity = json!({
        "file_id": Uuid::new_v4().to_string(),
        "sha256": "a".repeat(64),
        "size_bytes": 8,
        "endpoint_base_url": "http://docling.invalid/v1",
    });
    let deadline_at = Utc::now() + Duration::hours(1);

    let begun = db
        .begin_docling_submission(DoclingSubmissionIntent {
            fence: fence.clone(),
            endpoint_base_url: "http://docling.invalid/v1",
            submission_identity: &identity,
            deadline_at,
        })
        .await
        .expect("persist the intent")
        .expect("the owned item accepts one intent");
    assert_eq!(begun.status, "submitting");
    assert!(
        begun.remote_task_id.is_none(),
        "no remote id exists before a dispatch"
    );
    assert!(!begun.dispatch_attempted);
    assert_eq!(begun.submission_identity, identity);
    assert_eq!(
        begun.submission_execution_task_id,
        Some(execution.task_id())
    );
    assert_eq!(begun.submission_generation, Some(execution.generation));
    assert_eq!(
        begun.endpoint_base_url.as_deref(),
        Some("http://docling.invalid/v1")
    );

    let duplicate = db
        .begin_docling_submission(DoclingSubmissionIntent {
            fence: fence.clone(),
            endpoint_base_url: "http://docling.invalid/v1",
            submission_identity: &identity,
            deadline_at,
        })
        .await
        .expect("second begin");
    assert!(
        duplicate.is_none(),
        "the unresolved unique index holds one open submission per item"
    );

    let closed = db
        .mark_docling_preparation_failed(
            begun.id,
            &fence,
            "docling preparation failed before dispatch",
        )
        .await
        .expect("close the not-dispatched intent");
    assert!(closed);
    let (status, remote, dispatch_attempted, _, _, stored_identity, _) =
        submission_state(&db, begun.id).await;
    assert_eq!(status, "failed");
    assert!(remote.is_none());
    assert!(
        !dispatch_attempted,
        "a preparation failure never marks the dispatch attempted"
    );
    assert_eq!(stored_identity, identity);

    let retried = db
        .begin_docling_submission(DoclingSubmissionIntent {
            fence: fence.clone(),
            endpoint_base_url: "http://docling.invalid/v1",
            submission_identity: &identity,
            deadline_at,
        })
        .await
        .expect("begin after the failed intent")
        .expect("a failed intent no longer blocks the item");
    assert!(
        db.mark_docling_dispatch_attempted(retried.id, &fence)
            .await
            .expect("mark the dispatch attempted")
    );
    let confirmed = db
        .confirm_docling_submission(
            retried.id,
            &fence,
            "known-remote-id",
            Utc::now() + Duration::seconds(2),
        )
        .await
        .expect("confirm the accepted submission")
        .expect("the captured owner may confirm");
    assert_eq!(confirmed.status, "pending");
    assert_eq!(confirmed.remote_task_id.as_deref(), Some("known-remote-id"));

    let downgraded = db
        .mark_docling_submission_uncertain(retried.id, &fence, Some("late failure"))
        .await
        .expect("late uncertain marker");
    assert!(
        !downgraded,
        "a known pending row is never downgraded to an unknown outcome"
    );
    let (status, remote, _, _, _, _, _) = submission_state(&db, retried.id).await;
    assert_eq!(status, "pending");
    assert_eq!(remote.as_deref(), Some("known-remote-id"));

    cleanup(&db, task_id, user_id).await;
}

/// An unknown outcome parks the item inertly: waiting on `docling` with no due
/// time, unchanged budget, and no known job for the adopt path.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn an_unknown_dispatch_outcome_parks_without_a_due_time() {
    let db = connect().await;
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id).await;
    let claimed = claim_item(&db, item_id).await;
    let (fence, execution) = fence_of(task_id, &claimed);
    let identity = json!({"sha256": "b".repeat(64), "size_bytes": 8});
    let begun = db
        .begin_docling_submission(DoclingSubmissionIntent {
            fence: fence.clone(),
            endpoint_base_url: "http://docling.invalid/v1",
            submission_identity: &identity,
            deadline_at: Utc::now() + Duration::hours(1),
        })
        .await
        .expect("persist the intent")
        .expect("the owned item accepts one intent");
    let budget_before = failure_budget(&db, item_id).await;
    assert!(
        db.mark_docling_dispatch_attempted(begun.id, &fence)
            .await
            .expect("mark the dispatch attempted")
    );

    let outcome = db
        .park_submission_uncertain(ParkSubmissionUncertainRequest {
            task_id,
            item_id,
            execution,
            attempt_id: claimed.attempt_id,
            job_id: begun.id,
            reason: UNCERTAIN_REASON,
        })
        .await
        .expect("park the unknown outcome");
    assert_eq!(outcome, DoclingSubmissionOutcome::Parked);

    let (status, remote, dispatch_attempted, _, _, _, last_error) =
        submission_state(&db, begun.id).await;
    assert_eq!(status, "submission_uncertain");
    assert!(remote.is_none());
    assert!(dispatch_attempted);
    assert_eq!(last_error.as_deref(), Some(UNCERTAIN_REASON));

    assert!(
        db.get_active_docling_remote_job_for_item(item_id)
            .await
            .expect("known job lookup")
            .is_none(),
        "an unknown outcome is never adopted by the known-id poll path"
    );
    let unresolved = db
        .get_unresolved_docling_remote_job_for_item(item_id)
        .await
        .expect("unresolved lookup")
        .expect("the unknown outcome stays unresolved");
    assert_eq!(unresolved.id, begun.id);
    let listed = db
        .list_unresolved_docling_submissions_for_tasks(&[task_id])
        .await
        .expect("list unresolved submissions");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].status, "submission_uncertain");
    assert!(listed[0].dispatch_attempted);

    let wait = sqlx::query_file!("tests/sql/task_execution/read_item_wait_state.sql", item_id)
        .fetch_one(db.pool())
        .await
        .expect("read the parked item");
    assert_eq!(wait.waiting_reason.as_deref(), Some("docling"));
    assert!(
        wait.next_attempt_at.is_none(),
        "an unknown outcome is never due automatically"
    );
    assert_eq!(
        wait.execution_failure_count, budget_before,
        "parking an unknown outcome never spends the failure budget"
    );

    cleanup(&db, task_id, user_id).await;
}

/// An interrupted `submitting` segment recovers as an uncertain park instead of
/// an automatic requeue, so a crashed dispatch is never resubmitted.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn an_interrupted_submitting_segment_recovers_as_an_inert_park() {
    let db = connect().await;
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id).await;
    let claimed = claim_item(&db, item_id).await;
    let (fence, execution) = fence_of(task_id, &claimed);
    let identity = json!({"sha256": "c".repeat(64), "size_bytes": 8});
    let begun = db
        .begin_docling_submission(DoclingSubmissionIntent {
            fence: fence.clone(),
            endpoint_base_url: "http://docling.invalid/v1",
            submission_identity: &identity,
            deadline_at: Utc::now() + Duration::hours(1),
        })
        .await
        .expect("persist the intent")
        .expect("the owned item accepts one intent");
    assert!(
        db.mark_docling_dispatch_attempted(begun.id, &fence)
            .await
            .expect("mark the dispatch attempted")
    );
    let budget_before = failure_budget(&db, item_id).await;

    crash_holder(&db, &claimed).await;
    let mut tx = db.pool().begin().await.expect("recovery transaction");
    db.recover_expired_executions(&mut tx, 16)
        .await
        .expect("recover the expired segment");
    tx.commit().await.expect("commit the recovery");

    let (status, remote, dispatch_attempted, stored_task, stored_generation, _, last_error) =
        submission_state(&db, begun.id).await;
    assert_eq!(status, "submission_uncertain");
    assert!(remote.is_none());
    assert!(dispatch_attempted, "the invoked dispatch stays diagnostic");
    assert_eq!(stored_task, Some(execution.task_id()));
    assert_eq!(stored_generation, Some(execution.generation));
    assert!(last_error.is_some());

    let wait = sqlx::query_file!("tests/sql/task_execution/read_item_wait_state.sql", item_id)
        .fetch_one(db.pool())
        .await
        .expect("read the recovered item");
    assert_eq!(wait.waiting_reason.as_deref(), Some("docling"));
    assert!(wait.next_attempt_at.is_none());
    assert_eq!(
        wait.execution_failure_count, budget_before,
        "an interrupted dispatch is not a provider failure"
    );

    cleanup(&db, task_id, user_id).await;
}

fn pdf_source_payload() -> Value {
    json!({
        "filename": "doc.pdf",
        "media_type": "application/pdf",
        "options": { "metadata": {}, "source_policy": "retain" },
        "section_payload": [{ "section_key": "document", "body_text": "stale saved parse" }],
        "pipeline_progress": { "stage": "indexing", "attempt": 1 }
    })
}

async fn point_settings_at(db: &Database, base_url: &str) {
    db.save_docling_settings(&StoredDoclingSettings {
        base_url: base_url.to_string(),
        timeout_secs: 30,
        poll_interval_secs: 1,
        task_timeout_secs: 600,
        max_inflight: 1,
        pdf_backend: None,
        images_scale: None,
        image_export_mode: None,
        do_ocr: false,
        force_ocr: false,
        ocr_engine: None,
        ocr_lang: Vec::new(),
        do_code_enrichment: false,
        do_formula_enrichment: false,
        do_picture_description: false,
        openai_base_url: None,
        api_key: None,
        vlm_pipeline_model: None,
        picture_description_model: None,
        code_formula_model: None,
        picture_description_preset: None,
    })
    .await
    .expect("seed the docling settings");
}

/// A local Docling endpoint that accepts the POST and answers with an
/// unusable acknowledgement: the dispatch happened, the outcome is unknown.
async fn start_lost_response_docling(calls: Arc<AtomicUsize>) -> String {
    use axum::{Json, Router, routing::post};

    let router = Router::new().route(
        "/v1/convert/file/async",
        post(move || {
            let calls = calls.clone();
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Json(json!({ "task_id": "not a valid id" }))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind the local docling fake");
    let address = listener.local_addr().expect("address");
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    format!("http://{address}")
}

async fn fail_item_and_recompute(db: &Database, task_id: Uuid, item_id: Uuid) {
    sqlx::query(
        "UPDATE context69.task_items SET status = 'failed', failure_stage = 'parsing', \
         error_message = 'fixture failure', finished_at = now() WHERE id = $1",
    )
    .bind(item_id)
    .execute(db.pool())
    .await
    .expect("fail the fixture item");
    db.recompute_task(task_id)
        .await
        .expect("recompute the parent");
}

async fn seed_uncertain_job(db: &Database, task_id: Uuid, item_id: Uuid) -> Uuid {
    let job_id: Uuid = sqlx::query_file_scalar!(
        "tests/sql/docling_remote_jobs/seed_submitting_job.sql",
        task_id,
        item_id,
        false,
        Option::<Uuid>::None,
        Option::<i64>::None,
        json!({ "fixture": true }),
        "http://docling.invalid/v1",
        Utc::now() + Duration::hours(1),
    )
    .fetch_one(db.pool())
    .await
    .expect("seed the submitting job");
    sqlx::query(
        "UPDATE context69.task_docling_remote_jobs SET status = 'submission_uncertain', \
         last_error = 'seeded unknown outcome' WHERE id = $1",
    )
    .bind(job_id)
    .execute(db.pool())
    .await
    .expect("mark the seeded job uncertain");
    job_id
}

async fn task_snapshot(db: &Database, task_id: Uuid, item_id: Uuid) -> (String, String, i32, i64) {
    sqlx::query_as(
        "SELECT task.status, item.status, item.attempt_count, item.execution_generation \
         FROM context69.tasks task JOIN context69.task_items item ON item.task_id = task.id \
         WHERE task.id = $1 AND item.id = $2",
    )
    .bind(task_id)
    .bind(item_id)
    .fetch_one(db.pool())
    .await
    .expect("read the task snapshot")
}

async fn delete_tasks(db: &Database, task_ids: &[Uuid]) {
    let report =
        quiesce_fixture_executions(db.pool(), FixtureExecutionScope::Tasks(task_ids)).await;
    if let Ok(report) = report
        && report.has_expired_skips()
    {
        eprintln!("p5a fixture cleanup: expired grants remain for normal recovery: {report}");
    }
    for task_id in task_ids {
        sqlx::query_file!(
            "tests/sql/task_execution/delete_task_items_by_task.sql",
            task_id
        )
        .execute(db.pool())
        .await
        .expect("delete the fixture items");
        sqlx::query_file!("tests/sql/task_execution/delete_task_by_id.sql", task_id)
            .execute(db.pool())
            .await
            .expect("delete the fixture task");
    }
}

async fn delete_user(db: &Database, user_id: i64) {
    sqlx::query_file!(
        "tests/sql/task_execution/delete_user_idempotency_keys.sql",
        user_id
    )
    .execute(db.pool())
    .await
    .expect("delete the fixture idempotency keys");
    sqlx::query_file!("tests/sql/task_execution/delete_user.sql", user_id)
        .execute(db.pool())
        .await
        .expect("delete the fixture user");
}

async fn cleanup_tasks_and_user(db: &Database, task_ids: &[Uuid], user_id: i64) {
    delete_tasks(db, task_ids).await;
    delete_user(db, user_id).await;
}

async fn cleanup_source_fixture(
    db: &Database,
    task_ids: &[Uuid],
    group_id: i64,
    user_id: i64,
    storage_root: &std::path::Path,
) {
    delete_tasks(db, task_ids).await;
    sqlx::query_file!(
        "tests/sql/docling_remote_jobs/delete_group_documents_by_group.sql",
        group_id
    )
    .execute(db.pool())
    .await
    .expect("delete the fixture group documents");
    sqlx::query_file!(
        "tests/sql/library_ingest_dependency_wait/delete_group_files.sql",
        group_id
    )
    .execute(db.pool())
    .await
    .expect("delete the fixture group files");
    sqlx::query_file!(
        "tests/sql/library_ingest_dependency_wait/delete_group_storage_objects.sql",
        group_id
    )
    .execute(db.pool())
    .await
    .expect("delete the fixture group storage objects");
    sqlx::query_file!(
        "tests/sql/library_ingest_dependency_wait/delete_group_folders.sql",
        group_id
    )
    .execute(db.pool())
    .await
    .expect("delete the fixture group folders");
    sqlx::query_file!(
        "tests/sql/library_ingest_dependency_wait/delete_group.sql",
        group_id
    )
    .execute(db.pool())
    .await
    .expect("delete the fixture group");
    delete_user(db, user_id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}

/// A whole batch is refused before any task mutates, and the refusal is the
/// canonical 409.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_batch_recovery_is_refused_whole_before_any_task_mutates() {
    let db = connect().await;
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (ordinary_task, ordinary_item) = seed_task(&db, user_id).await;
    fail_item_and_recompute(&db, ordinary_task, ordinary_item).await;
    let (uncertain_task, uncertain_item) = seed_task(&db, user_id).await;
    fail_item_and_recompute(&db, uncertain_task, uncertain_item).await;
    seed_uncertain_job(&db, uncertain_task, uncertain_item).await;

    let mock = QdrantPointsMock::start().await;
    let root = std::env::temp_dir().join(format!("p5a-batch-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).expect("storage root");
    let service = build_service(&db, &root, Arc::new(FakeEmbedding::new(0)), &mock, None).await;

    let before = task_snapshot(&db, ordinary_task, ordinary_item).await;
    let error = service
        .batch_retry(
            user_id,
            &TaskBatchRetryRequest {
                task_ids: vec![ordinary_task, uncertain_task],
                acknowledge_docling_duplicate_risk: false,
            },
        )
        .await
        .expect_err("a submitted-uncertain task refuses the whole batch");
    let domain = context69::domain_errors::find_domain_error(&error)
        .expect("the refusal carries the typed domain error");
    assert!(matches!(
        domain,
        context69::domain_errors::DomainError::DoclingSubmissionUncertain(_)
    ));
    assert_eq!(domain.status_code_number(), 409);
    assert_eq!(
        task_snapshot(&db, ordinary_task, ordinary_item).await,
        before,
        "a refused batch mutates no task"
    );

    cleanup_tasks_and_user(&db, &[ordinary_task, uncertain_task], user_id).await;
    let _ = std::fs::remove_dir_all(&root);
}

/// A task the caller cannot read never discloses its uncertain submission and
/// never refuses the batch: the ordinary per-item `not_found` stays.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_foreign_uncertain_submission_is_not_disclosed_by_batch_recovery() {
    let db = connect().await;
    let _guard = LOCK.lock().await;
    let owner_id = seed_user(&db).await;
    let (owner_task, owner_item) = seed_task(&db, owner_id).await;
    fail_item_and_recompute(&db, owner_task, owner_item).await;
    let foreign_id = seed_user(&db).await;
    let (foreign_task, foreign_item) = seed_task(&db, foreign_id).await;
    fail_item_and_recompute(&db, foreign_task, foreign_item).await;
    seed_uncertain_job(&db, foreign_task, foreign_item).await;

    let mock = QdrantPointsMock::start().await;
    let root = std::env::temp_dir().join(format!("p5a-batch-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).expect("storage root");
    let service = build_service(&db, &root, Arc::new(FakeEmbedding::new(0)), &mock, None).await;

    let response = service
        .batch_retry(
            owner_id,
            &TaskBatchRetryRequest {
                task_ids: vec![owner_task, foreign_task],
                acknowledge_docling_duplicate_risk: false,
            },
        )
        .await
        .expect("an unreadable uncertain task never refuses the batch");
    assert_eq!(response.results.len(), 2);
    assert_eq!(
        response.results[1].outcome,
        TaskBatchRetryOutcome::Skipped,
        "the foreign id stays an ordinary per-item refusal"
    );
    assert_eq!(
        response.results[1].reason,
        Some(TaskBatchRetryReason::NotFound)
    );
    assert_eq!(
        response.results[0].outcome,
        TaskBatchRetryOutcome::Accepted,
        "the ordinary owned task is still recovered"
    );

    cleanup_tasks_and_user(&db, &[owner_task], owner_id).await;
    cleanup_tasks_and_user(&db, &[foreign_task], foreign_id).await;
    let _ = std::fs::remove_dir_all(&root);
}

/// A real lost submission acknowledgement parks the item as uncertain, and a
/// current re-entry adopts the durable row without a second POST.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_lost_submission_response_parks_uncertain_and_reentry_never_resends() {
    let db = connect().await;
    let _guard = LOCK.lock().await;
    let user_id = source_rerun_support::seed_user(&db).await;
    let group = seed_group(&db).await;
    let file = seed_pdf_source_file(&db, &group).await;
    let submissions = Arc::new(AtomicUsize::new(0));
    let base_url = start_lost_response_docling(submissions.clone()).await;
    point_settings_at(&db, &base_url).await;
    let (source_task, _) = seed_succeeded_source(
        &db,
        user_id,
        &group,
        &file,
        pdf_source_payload(),
        "file_batch",
    )
    .await;
    let (new_task, new_items) = db
        .rerun_task_source(source_task)
        .await
        .expect("fork the PDF source");
    let item_id = new_items[0];
    let mock = QdrantPointsMock::start().await;
    let service = build_service(
        &db,
        &file.storage_root,
        Arc::new(FakeEmbedding::new(0)),
        &mock,
        Some([1000, 1000, 1000]),
    )
    .await;

    let first = source_rerun_support::claim_item(&db, new_task).await;
    service
        .run_item_for_test(first)
        .await
        .expect("the lost acknowledgement is a typed outcome, not an error");
    assert_eq!(
        submissions.load(Ordering::SeqCst),
        1,
        "the dispatch was invoked exactly once"
    );
    let job = db
        .get_unresolved_docling_remote_job_for_item(item_id)
        .await
        .expect("unresolved lookup")
        .expect("the unknown outcome stays durable");
    assert_eq!(job.status, "submission_uncertain");
    assert!(
        job.dispatch_attempted,
        "the invoked dispatch stays diagnostic"
    );
    assert!(job.remote_task_id.is_none());
    let wait = sqlx::query_file!("tests/sql/task_execution/read_item_wait_state.sql", item_id)
        .fetch_one(db.pool())
        .await
        .expect("read the parked item");
    assert_eq!(wait.waiting_reason.as_deref(), Some("docling"));
    assert!(wait.next_attempt_at.is_none());

    sqlx::query(
        "UPDATE context69.task_items SET status = 'queued', waiting_reason = NULL, \
         next_attempt_at = now(), updated_at = now() WHERE id = $1",
    )
    .bind(item_id)
    .execute(db.pool())
    .await
    .expect("make the fixture item claimable again");
    let second = source_rerun_support::claim_item(&db, new_task).await;
    service
        .run_item_for_test(second)
        .await
        .expect("the re-entry adopts the durable unknown outcome");
    assert_eq!(
        submissions.load(Ordering::SeqCst),
        1,
        "a re-entry never resends an unknown dispatch"
    );
    let again = db
        .get_unresolved_docling_remote_job_for_item(item_id)
        .await
        .expect("unresolved lookup")
        .expect("the unknown outcome is still durable");
    assert_eq!(again.id, job.id);
    assert_eq!(again.status, "submission_uncertain");
    let wait = sqlx::query_file!("tests/sql/task_execution/read_item_wait_state.sql", item_id)
        .fetch_one(db.pool())
        .await
        .expect("read the re-parked item");
    assert_eq!(wait.waiting_reason.as_deref(), Some("docling"));
    assert!(wait.next_attempt_at.is_none());

    cleanup_source_fixture(
        &db,
        &[new_task, source_task],
        group.id,
        user_id,
        &file.storage_root,
    )
    .await;
}

/// An interrupted intent that never marked the dispatch still recovers as an
/// inert uncertain park, with the diagnostic flag preserved.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn an_unmarked_interrupted_submitting_segment_also_recovers_as_an_inert_park() {
    let db = connect().await;
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id).await;
    let claimed = claim_item(&db, item_id).await;
    let (fence, execution) = fence_of(task_id, &claimed);
    let job_id: Uuid = sqlx::query_file_scalar!(
        "tests/sql/docling_remote_jobs/seed_submitting_job.sql",
        task_id,
        item_id,
        false,
        Some(execution.task_id()),
        Some(execution.generation),
        json!({ "sha256": "d".repeat(64), "size_bytes": 8 }),
        "http://docling.invalid/v1",
        Utc::now() + Duration::hours(1),
    )
    .fetch_one(db.pool())
    .await
    .expect("seed the unmarked intent");
    let budget_before = failure_budget(&db, item_id).await;
    crash_holder(&db, &claimed).await;
    let mut tx = db.pool().begin().await.expect("recovery transaction");
    db.recover_expired_executions(&mut tx, 16)
        .await
        .expect("recover the expired segment");
    tx.commit().await.expect("commit the recovery");

    let (status, remote, dispatch_attempted, _, _, _, _) = submission_state(&db, job_id).await;
    assert_eq!(status, "submission_uncertain");
    assert!(remote.is_none());
    assert!(
        !dispatch_attempted,
        "an unmarked intent keeps its cleared diagnostic flag"
    );
    let wait = sqlx::query_file!("tests/sql/task_execution/read_item_wait_state.sql", item_id)
        .fetch_one(db.pool())
        .await
        .expect("read the recovered item");
    assert_eq!(wait.waiting_reason.as_deref(), Some("docling"));
    assert!(wait.next_attempt_at.is_none());
    assert_eq!(wait.execution_failure_count, budget_before);
    let _ = fence;

    cleanup(&db, task_id, user_id).await;
}

/// A foreign owner cannot resolve a still-current intent; a new owner may
/// resolve exactly the stale one it replaced.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_foreign_or_stale_owner_cannot_resolve_a_current_intent() {
    let db = connect().await;
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id).await;
    let claimed = claim_item(&db, item_id).await;
    let (fence, execution) = fence_of(task_id, &claimed);
    let other_execution = Uuid::new_v4();
    let job_id: Uuid = sqlx::query_file_scalar!(
        "tests/sql/docling_remote_jobs/seed_submitting_job.sql",
        task_id,
        item_id,
        false,
        Some(other_execution),
        Some(execution.generation),
        json!({ "sha256": "e".repeat(64), "size_bytes": 8 }),
        "http://docling.invalid/v1",
        Utc::now() + Duration::hours(1),
    )
    .fetch_one(db.pool())
    .await
    .expect("seed the submitting intent");
    sqlx::query_file!(
        "tests/sql/docling_remote_jobs/set_submission_owner.sql",
        job_id,
        execution.task_id(),
        execution.generation,
    )
    .fetch_one(db.pool())
    .await
    .expect("bind the intent to the current owner");

    let foreign = DoclingSubmissionFence {
        task_id,
        item_id,
        lease_token: claimed.lease_token,
        execution_task_id: other_execution,
        generation: execution.generation + 1,
    };
    assert!(
        !db.mark_docling_submission_uncertain(job_id, &foreign, Some("foreign owner"))
            .await
            .expect("foreign marker"),
        "a foreign owner never resolves an intent it did not create"
    );
    let (status, _, _, _, _, _, _) = submission_state(&db, job_id).await;
    assert_eq!(status, "submitting");

    sqlx::query_file!(
        "tests/sql/docling_remote_jobs/set_submission_owner.sql",
        job_id,
        other_execution,
        execution.generation + 1,
    )
    .fetch_one(db.pool())
    .await
    .expect("bind the intent to a superseded owner");
    assert!(
        db.mark_docling_submission_uncertain(job_id, &fence, Some("stale adoption"))
            .await
            .expect("stale marker"),
        "the new captured owner may resolve the stale intent it replaced"
    );
    let (status, _, _, _, _, _, _) = submission_state(&db, job_id).await;
    assert_eq!(status, "submission_uncertain");

    cleanup(&db, task_id, user_id).await;
}

/// A real library cancellation wins the uncertain park without changing the
/// durable intent or the failure budget.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_library_cancellation_wins_without_changing_the_intent() {
    let db = connect().await;
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id).await;
    let claimed = claim_item(&db, item_id).await;
    let (fence, execution) = fence_of(task_id, &claimed);
    let job_id: Uuid = sqlx::query_file_scalar!(
        "tests/sql/docling_remote_jobs/seed_submitting_job.sql",
        task_id,
        item_id,
        true,
        Some(execution.task_id()),
        Some(execution.generation),
        json!({ "sha256": "f".repeat(64), "size_bytes": 8 }),
        "http://docling.invalid/v1",
        Utc::now() + Duration::hours(1),
    )
    .fetch_one(db.pool())
    .await
    .expect("seed the submitting intent");
    let budget_before = failure_budget(&db, item_id).await;
    db.execution_queue()
        .await
        .expect("load the execution queue")
        .request_cancel(TaskId::from_uuid(execution.task_id()))
        .await
        .expect("request the library cancellation");

    let outcome = db
        .park_submission_uncertain(ParkSubmissionUncertainRequest {
            task_id,
            item_id,
            execution,
            attempt_id: claimed.attempt_id,
            job_id,
            reason: UNCERTAIN_REASON,
        })
        .await
        .expect("classify the cancelled uncertain park");
    assert_eq!(outcome, DoclingSubmissionOutcome::Cancelled);
    let wait = sqlx::query_file!("tests/sql/task_execution/read_item_wait_state.sql", item_id)
        .fetch_one(db.pool())
        .await
        .expect("read the cancelled item");
    assert_eq!(wait.execution_failure_count, budget_before);
    let item_status: String =
        sqlx::query_scalar("SELECT status FROM context69.task_items WHERE id = $1")
            .bind(item_id)
            .fetch_one(db.pool())
            .await
            .expect("read the cancelled item status");
    assert_eq!(item_status, "cancelled");
    let (status, _, dispatch_attempted, _, _, _, _) = submission_state(&db, job_id).await;
    assert_eq!(
        status, "submitting",
        "cancellation never rewrites the durable unknown intake"
    );
    assert!(dispatch_attempted);
    let _ = fence;

    cleanup(&db, task_id, user_id).await;
}

/// An acknowledged retry archives the settled unknown submission and reopens
/// its waiting item in one commit, keeping the diagnostic and the ordinal.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn an_acknowledged_retry_archives_and_reopens_a_settled_uncertain_item() {
    let db = connect().await;
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id).await;
    let claimed = claim_item(&db, item_id).await;
    let attempt_count = claimed.attempt_count;
    let (fence, execution) = fence_of(task_id, &claimed);
    let begun = db
        .begin_docling_submission(DoclingSubmissionIntent {
            fence: fence.clone(),
            endpoint_base_url: "http://docling.invalid/v1",
            submission_identity: &json!({ "sha256": "1".repeat(64), "size_bytes": 8 }),
            deadline_at: Utc::now() + Duration::hours(1),
        })
        .await
        .expect("persist the intent")
        .expect("the owned item accepts one intent");
    assert!(
        db.mark_docling_dispatch_attempted(begun.id, &fence)
            .await
            .expect("mark the dispatch attempted")
    );
    let parked = db
        .park_submission_uncertain(ParkSubmissionUncertainRequest {
            task_id,
            item_id,
            execution,
            attempt_id: claimed.attempt_id,
            job_id: begun.id,
            reason: UNCERTAIN_REASON,
        })
        .await
        .expect("park the unknown outcome");
    assert_eq!(parked, DoclingSubmissionOutcome::Parked);
    let before = submission_state(&db, begun.id).await;
    assert_eq!(before.0, "submission_uncertain");

    let mock = QdrantPointsMock::start().await;
    let root = std::env::temp_dir().join(format!("p5a-ack-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).expect("storage root");
    let service = build_service(&db, &root, Arc::new(FakeEmbedding::new(0)), &mock, None).await;
    let response = service
        .retry_with_ack(task_id, user_id, true)
        .await
        .expect("an acknowledged settled retry is accepted");
    assert_eq!(response.retried_items, 1);
    assert_eq!(response.task.item_ids, vec![item_id]);

    let after = submission_state(&db, begun.id).await;
    assert_eq!(after.0, "superseded", "the old diagnostic is retained");
    assert_eq!(after.2, before.2, "the dispatch flag survives the archive");
    assert_eq!(after.3, before.3, "the execution provenance survives");
    assert_eq!(after.4, before.4);
    assert_eq!(after.5, before.5, "the frozen identity survives");
    let reopened = sqlx::query_file!(
        "tests/sql/task_execution/read_item_retry_state.sql",
        item_id
    )
    .fetch_one(db.pool())
    .await
    .expect("read the reopened item");
    assert_eq!(reopened.status, "queued");
    assert_eq!(
        reopened.execution_failure_count, 0,
        "a manual retry restarts the failure budget"
    );
    assert_eq!(
        reopened.attempt_count, attempt_count,
        "the claim ordinal stays monotonic"
    );

    cleanup(&db, task_id, user_id).await;
    let _ = std::fs::remove_dir_all(&root);
}

/// An acknowledged retry never overrides a live dispatch, whichever value the
/// diagnostic flag carries.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn an_acknowledged_retry_refuses_a_live_submitting_intent() {
    let db = connect().await;
    let _guard = LOCK.lock().await;
    let mock = QdrantPointsMock::start().await;
    let root = std::env::temp_dir().join(format!("p5a-ack-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).expect("storage root");
    let service = build_service(&db, &root, Arc::new(FakeEmbedding::new(0)), &mock, None).await;
    for dispatch_attempted in [false, true] {
        let user_id = seed_user(&db).await;
        let (task_id, item_id) = seed_task(&db, user_id).await;
        let claimed = claim_item(&db, item_id).await;
        let (fence, _) = fence_of(task_id, &claimed);
        let begun = db
            .begin_docling_submission(DoclingSubmissionIntent {
                fence: fence.clone(),
                endpoint_base_url: "http://docling.invalid/v1",
                submission_identity: &json!({ "sha256": "2".repeat(64), "size_bytes": 8 }),
                deadline_at: Utc::now() + Duration::hours(1),
            })
            .await
            .expect("persist the intent")
            .expect("the owned item accepts one intent");
        if dispatch_attempted {
            assert!(
                db.mark_docling_dispatch_attempted(begun.id, &fence)
                    .await
                    .expect("mark the dispatch attempted")
            );
        }
        let error = service
            .retry_with_ack(task_id, user_id, true)
            .await
            .expect_err("a live dispatch is never acknowledged");
        let domain = context69::domain_errors::find_domain_error(&error).expect("typed refusal");
        assert!(matches!(
            domain,
            context69::domain_errors::DomainError::DoclingSubmissionUncertain(_)
        ));
        assert_eq!(domain.status_code_number(), 409);
        let (status, _, stamped, _, _, _, _) = submission_state(&db, begun.id).await;
        assert_eq!(status, "submitting");
        assert_eq!(stamped, dispatch_attempted);
        cleanup(&db, task_id, user_id).await;
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// Without acknowledgement a settled unknown submission still refuses the
/// single retry with the code-only conflict and mutates nothing.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn an_unacknowledged_retry_refuses_a_settled_uncertain_item_without_mutation() {
    let db = connect().await;
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id).await;
    let claimed = claim_item(&db, item_id).await;
    let (fence, execution) = fence_of(task_id, &claimed);
    let begun = db
        .begin_docling_submission(DoclingSubmissionIntent {
            fence: fence.clone(),
            endpoint_base_url: "http://docling.invalid/v1",
            submission_identity: &json!({ "sha256": "3".repeat(64), "size_bytes": 8 }),
            deadline_at: Utc::now() + Duration::hours(1),
        })
        .await
        .expect("persist the intent")
        .expect("the owned item accepts one intent");
    assert!(
        db.mark_docling_dispatch_attempted(begun.id, &fence)
            .await
            .expect("mark the dispatch attempted")
    );
    let parked = db
        .park_submission_uncertain(ParkSubmissionUncertainRequest {
            task_id,
            item_id,
            execution,
            attempt_id: claimed.attempt_id,
            job_id: begun.id,
            reason: UNCERTAIN_REASON,
        })
        .await
        .expect("park the unknown outcome");
    assert_eq!(parked, DoclingSubmissionOutcome::Parked);

    let mock = QdrantPointsMock::start().await;
    let root = std::env::temp_dir().join(format!("p5b1-unack-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).expect("storage root");
    let service = build_service(&db, &root, Arc::new(FakeEmbedding::new(0)), &mock, None).await;
    let before = submission_state(&db, begun.id).await;
    let before_task = task_snapshot(&db, task_id, item_id).await;

    let error = service
        .retry(task_id, user_id)
        .await
        .expect_err("an unacknowledged settled submission is refused");
    let domain = context69::domain_errors::find_domain_error(&error).expect("typed refusal");
    assert!(matches!(
        domain,
        context69::domain_errors::DomainError::DoclingSubmissionUncertain(_)
    ));
    assert_eq!(domain.status_code_number(), 409);
    assert_eq!(
        submission_state(&db, begun.id).await,
        before,
        "the old diagnostic is untouched"
    );
    assert_eq!(
        task_snapshot(&db, task_id, item_id).await,
        before_task,
        "the parked item is not reopened"
    );

    cleanup(&db, task_id, user_id).await;
    let _ = std::fs::remove_dir_all(&root);
}

/// An acknowledged cancelled resume archives the settled unknown submission
/// and reopens the cancelled item in the same transaction.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn an_acknowledged_cancelled_resume_archives_and_reopens_the_settled_item() {
    let db = connect().await;
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id).await;
    let claimed = claim_item(&db, item_id).await;
    let attempt_count = claimed.attempt_count;
    let job_id = seed_uncertain_job(&db, task_id, item_id).await;
    sqlx::query(
        "UPDATE context69.task_items SET status = 'cancelled', finished_at = now(), \
         lease_token = NULL, lease_until = NULL WHERE id = $1",
    )
    .bind(item_id)
    .execute(db.pool())
    .await
    .expect("cancel the fixture item");
    sqlx::query(
        "UPDATE context69.tasks SET status = 'cancelled', cancelled_count = 1, \
         queued_count = 0, finished_at = now() WHERE id = $1",
    )
    .bind(task_id)
    .execute(db.pool())
    .await
    .expect("cancel the fixture task");
    let before = submission_state(&db, job_id).await;

    let resumed = db
        .resume_cancelled_task_items_with_ack(task_id, user_id, DoclingRiskAck::Acknowledged)
        .await
        .expect("the acknowledged cancelled resume is accepted");
    assert_eq!(resumed, vec![item_id]);

    let after = submission_state(&db, job_id).await;
    assert_eq!(after.0, "superseded", "the old diagnostic is retained");
    assert_eq!(after.2, before.2, "the dispatch flag survives the archive");
    assert_eq!(after.5, before.5, "the frozen identity survives");
    let reopened = sqlx::query_file!(
        "tests/sql/task_execution/read_item_retry_state.sql",
        item_id
    )
    .fetch_one(db.pool())
    .await
    .expect("read the reopened item");
    assert_eq!(reopened.status, "queued");
    assert_eq!(reopened.execution_failure_count, 0);
    assert_eq!(reopened.attempt_count, attempt_count);

    cleanup(&db, task_id, user_id).await;
}

type SubmissionHistoryRow = (
    Uuid,
    Uuid,
    String,
    bool,
    Value,
    Option<String>,
    chrono::DateTime<Utc>,
    Option<chrono::DateTime<Utc>>,
    String,
    Option<String>,
);

async fn submission_history(db: &Database, task_id: Uuid) -> Vec<SubmissionHistoryRow> {
    sqlx::query_file!(
        "tests/sql/docling_remote_jobs/read_submission_history.sql",
        task_id
    )
    .fetch_all(db.pool())
    .await
    .expect("read the retained submission history")
    .into_iter()
    .map(|row| {
        (
            row.id,
            row.item_id,
            row.job_status,
            row.dispatch_attempted,
            row.submission_identity,
            row.last_error,
            row.created_at,
            row.finished_at,
            row.item_status,
            row.item_waiting_reason,
        )
    })
    .collect()
}

/// A task with two distinct items: one becomes the settled unknown submission,
/// the other keeps an unrelated known job.
async fn seed_two_item_task(db: &Database, user_id: i64) -> (Uuid, Uuid, Uuid) {
    let task_id = Uuid::new_v4();
    let (_, _, item_ids) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id,
            user_id,
            group_id: None,
            kind: "text_batch",
            group_path: Some("test/p5b1v"),
            source_key: None,
            payloads: &[
                json!({"external_id": "uncertain"}),
                json!({"external_id": "known"}),
            ],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: &format!("p5b1v-two-item-{}", Uuid::new_v4()),
        })
        .await
        .expect("create the two-item task");
    (task_id, item_ids[0], item_ids[1])
}

async fn seed_known_job(
    db: &Database,
    task_id: Uuid,
    item_id: Uuid,
    next_poll_at: chrono::DateTime<Utc>,
    deadline_at: Option<chrono::DateTime<Utc>>,
) -> Uuid {
    let job_id = Uuid::new_v4();
    sqlx::query_file_scalar!(
        "tests/sql/docling_remote_jobs/seed_known_job.sql",
        job_id,
        task_id,
        item_id,
        format!("remote-known-{}", Uuid::new_v4()),
        Some("pending".to_string()),
        0_i32,
        0_i32,
        next_poll_at,
        deadline_at,
        Some("http://known.invalid/v1".to_string()),
    )
    .fetch_one(db.pool())
    .await
    .expect("seed the unrelated known job")
}

fn processing_query() -> TaskListQuery {
    TaskListQuery {
        page: 1,
        page_size: 100,
        query: None,
        group_path: None,
        source_key: None,
        kind: None,
        status: None,
        view: Some(TaskListView::Processing),
        stage: None,
        waiting_reason: None,
        dependency_key: None,
        sort_by: None,
        sort_direction: None,
    }
}

async fn parked_fixture(
    db: &Database,
    task_id: Uuid,
    item_id: Uuid,
    identity: Value,
) -> (Uuid, i32) {
    let claimed = claim_item(db, item_id).await;
    let attempt_count = claimed.attempt_count;
    let (fence, execution) = fence_of(task_id, &claimed);
    let begun = db
        .begin_docling_submission(DoclingSubmissionIntent {
            fence: fence.clone(),
            endpoint_base_url: "http://docling.invalid/v1",
            submission_identity: &identity,
            deadline_at: Utc::now() + Duration::hours(1),
        })
        .await
        .expect("persist the intent")
        .expect("the owned item accepts one intent");
    assert!(
        db.mark_docling_dispatch_attempted(begun.id, &fence)
            .await
            .expect("mark the dispatch attempted")
    );
    let parked = db
        .park_submission_uncertain(ParkSubmissionUncertainRequest {
            task_id,
            item_id,
            execution,
            attempt_id: claimed.attempt_id,
            job_id: begun.id,
            reason: UNCERTAIN_REASON,
        })
        .await
        .expect("park the unknown outcome");
    assert_eq!(parked, DoclingSubmissionOutcome::Parked);
    (begun.id, attempt_count)
}

/// Always settles the scoped fixture and removes its artifact root, then
/// resumes the body's panic, so a failing assertion never leaves a live grant
/// or a second fixture behind for the next case.
async fn with_scoped_teardown<F>(
    db: &Database,
    task_id: Uuid,
    user_id: i64,
    root: &std::path::Path,
    body: F,
) where
    F: std::future::Future<Output = ()>,
{
    use futures::FutureExt;
    use std::panic::AssertUnwindSafe;

    let caught = AssertUnwindSafe(body).catch_unwind().await;
    let teardown = AssertUnwindSafe(async {
        cleanup(db, task_id, user_id).await;
        let _ = std::fs::remove_dir_all(root);
    })
    .catch_unwind()
    .await;
    match caught {
        Ok(()) => {
            if let Err(payload) = teardown {
                std::panic::resume_unwind(payload);
            }
        }
        Err(payload) => {
            if teardown.is_err() {
                eprintln!("scoped teardown failed after the fixture body panicked");
            }
            std::panic::resume_unwind(payload);
        }
    }
}

/// Like [`with_scoped_teardown`], for cases whose body creates more than one
/// predeclared fixture task. Every listed task is quiesced and removed on both
/// success and panic, so no failing assertion leaks rows into the next case.
async fn with_tasks_teardown<F>(
    db: &Database,
    task_ids: &[Uuid],
    user_id: i64,
    root: &std::path::Path,
    body: F,
) where
    F: std::future::Future<Output = ()>,
{
    use futures::FutureExt;
    use std::panic::AssertUnwindSafe;

    let caught = AssertUnwindSafe(body).catch_unwind().await;
    let teardown = AssertUnwindSafe(async {
        cleanup_tasks_and_user(db, task_ids, user_id).await;
        let _ = std::fs::remove_dir_all(root);
    })
    .catch_unwind()
    .await;
    match caught {
        Ok(()) => {
            if let Err(payload) = teardown {
                std::panic::resume_unwind(payload);
            }
        }
        Err(payload) => {
            if teardown.is_err() {
                eprintln!("multi-task teardown failed after the fixture body panicked");
            }
            std::panic::resume_unwind(payload);
        }
    }
}

/// Create one text fixture task with a predeclared id so the teardown guard can
/// remove it even when a later fixture step panics.
async fn create_fixture_task(
    db: &Database,
    user_id: i64,
    task_id: Uuid,
    external_id: &str,
) -> Uuid {
    let payloads = [json!({ "external_id": external_id })];
    let (_, _, items) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id,
            user_id,
            group_id: None,
            kind: "text_batch",
            group_path: Some("test/docling-remote-jobs"),
            source_key: None,
            payloads: &payloads,
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: &format!("r1-fixture-{task_id}"),
        })
        .await
        .expect("create fixture task");
    items[0]
}

/// Two concurrent acknowledged retries of one settled item archive and reopen
/// it exactly once, and the losing request never double-enqueues or regresses
/// the flag/history.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn concurrent_acknowledged_retries_archive_and_reopen_only_once() {
    let db = connect().await;
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id).await;
    let root = std::env::temp_dir().join(format!("p5b1v-concurrent-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).expect("storage root");
    let body_root = root.clone();
    let db = &db;
    with_scoped_teardown(db, task_id, user_id, &root, async move {
        let (_, attempt_count) = parked_fixture(
            db,
            task_id,
            item_id,
            json!({ "sha256": "c".repeat(64), "marker": "p5b1v-concurrent" }),
        )
        .await;

        let mock = QdrantPointsMock::start().await;
        let service =
            build_service(db, &body_root, Arc::new(FakeEmbedding::new(0)), &mock, None).await;

        let (first, second) = tokio::join!(
            service.retry_with_ack(task_id, user_id, true),
            service.retry_with_ack(task_id, user_id, true),
        );
        let mut winners = 0;
        for result in [first, second] {
            match result {
                Ok(response) => {
                    if response.retried_items == 1 {
                        winners += 1;
                    }
                }
                Err(error) => {
                    let domain =
                        context69::domain_errors::find_domain_error(&error).expect("typed refusal");
                    assert!(
                        matches!(
                            domain,
                            context69::domain_errors::DomainError::InvalidArgument(_)
                        ),
                        "a losing concurrent request has nothing left to recover: {domain:?}"
                    );
                    assert_eq!(
                        domain.status_code_number(),
                        400,
                        "a repeated acknowledgement is a normal invalid argument"
                    );
                }
            }
        }
        assert_eq!(winners, 1, "exactly one request reopens the item");

        let history = submission_history(db, task_id).await;
        assert_eq!(history.len(), 1, "one archive row, never a duplicate");
        assert_eq!(history[0].1, item_id);
        assert_eq!(history[0].2, "superseded");
        assert!(history[0].7.is_some(), "the archive is finished");
        assert_eq!(history[0].8, "queued", "the one item is reopened once");
        let task = service.get(task_id, user_id).await.expect("read the task");
        assert!(
            !task.docling_submission_uncertain,
            "the winning archive does not regress the risk flag"
        );
        let item_rows: i64 =
            sqlx::query_scalar("SELECT count(*) FROM context69.task_items WHERE task_id = $1")
                .bind(task_id)
                .fetch_one(db.pool())
                .await
                .expect("count the fixture items");
        assert_eq!(item_rows, 1, "the losing request enqueues nothing");
        let reopened = sqlx::query_file!(
            "tests/sql/task_execution/read_item_retry_state.sql",
            item_id
        )
        .fetch_one(db.pool())
        .await
        .expect("read the reopened item");
        assert_eq!(
            reopened.attempt_count, attempt_count,
            "the claim ordinal stays monotonic across both requests"
        );
    })
    .await;
}

/// After an acknowledged reopen, a real new dispatch intent gets its own job id
/// while the archived diagnostic keeps its identity/provenance/timestamp, and a
/// second acknowledgement against the live intent is still refused.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn acknowledged_reopen_creates_a_distinct_future_intent() {
    let db = connect().await;
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id).await;
    let root = std::env::temp_dir().join(format!("p5b1v-history-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).expect("storage root");
    let body_root = root.clone();
    let db = &db;
    with_scoped_teardown(db, task_id, user_id, &root, async move {
        let (old_job, _) = parked_fixture(
            db,
            task_id,
            item_id,
            json!({ "sha256": "d".repeat(64), "marker": "p5b1v-history" }),
        )
        .await;

        let mock = QdrantPointsMock::start().await;
        let service =
            build_service(db, &body_root, Arc::new(FakeEmbedding::new(0)), &mock, None).await;

        let response = service
            .retry_with_ack(task_id, user_id, true)
            .await
            .expect("the acknowledged retry is accepted");
        assert_eq!(response.retried_items, 1);
        let archived = submission_history(db, task_id).await;
        assert_eq!(archived.len(), 1);
        let old_identity = archived[0].4.clone();
        let old_created_at = archived[0].6;
        let old_state = submission_state(db, old_job).await;

        let reclaimed = claim_item(db, item_id).await;
        let (fence, _) = fence_of(task_id, &reclaimed);
        let renewed = db
            .begin_docling_submission(DoclingSubmissionIntent {
                fence,
                endpoint_base_url: "http://docling.invalid/v1",
                submission_identity: &json!({ "sha256": "e".repeat(64) }),
                deadline_at: Utc::now() + Duration::hours(1),
            })
            .await
            .expect("persist the distinct future intent")
            .expect("the reopened item accepts a new intent");
        assert_ne!(renewed.id, old_job, "the future intent has its own job id");
        assert_eq!(renewed.status, "submitting");

        let history = submission_history(db, task_id).await;
        assert_eq!(history.len(), 2, "old diagnostic plus the distinct intent");
        let old_row = history
            .iter()
            .find(|row| row.0 == old_job)
            .expect("the archived row survives");
        assert_eq!(old_row.2, "superseded");
        assert_eq!(old_row.4, old_identity, "identity is preserved");
        assert_eq!(old_row.6, old_created_at, "created_at is preserved");
        let old_after = submission_state(db, old_job).await;
        assert_eq!(
            old_after.3, old_state.3,
            "execution provenance is preserved"
        );
        assert_eq!(
            old_after.4, old_state.4,
            "submission generation is preserved"
        );
        assert_eq!(old_after.5, old_state.5, "frozen identity is preserved");
        let live_row = history
            .iter()
            .find(|row| row.0 == renewed.id)
            .expect("the new intent is retained");
        assert_eq!(live_row.2, "submitting");
        assert!(
            !live_row.3,
            "no dispatch attempt is claimed for a prepared intent"
        );

        let task = service.get(task_id, user_id).await.expect("read the task");
        assert!(
            task.docling_submission_uncertain,
            "the live intent raises the risk flag again"
        );
        let diagnose = service
            .diagnose(task_id, user_id)
            .await
            .expect("diagnose the task");
        assert_eq!(
            diagnose.items[0]
                .docling_submission
                .as_ref()
                .expect("the item view is present")
                .state,
            TaskDoclingSubmissionState::Submitting
        );

        let before = submission_state(db, renewed.id).await;
        let error = service
            .retry_with_ack(task_id, user_id, true)
            .await
            .expect_err("a live intent is never acknowledged");
        let domain = context69::domain_errors::find_domain_error(&error).expect("typed refusal");
        assert_eq!(domain.status_code_number(), 409);
        assert_eq!(
            submission_state(db, renewed.id).await,
            before,
            "the live intent is unchanged by the refusal"
        );
    })
    .await;
}

/// Acknowledging one item never refuses or mutates an unrelated known job on
/// the same task.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn an_acknowledged_retry_leaves_an_unrelated_known_job_untouched() {
    let db = connect().await;
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, uncertain_item, known_item) = seed_two_item_task(&db, user_id).await;
    let root = std::env::temp_dir().join(format!("p5b1v-known-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).expect("storage root");
    let body_root = root.clone();
    let db = &db;
    with_scoped_teardown(db, task_id, user_id, &root, async move {
        sqlx::query(
            "UPDATE context69.task_items SET status = 'waiting', waiting_reason = 'docling', \
             stage = 'docling', finished_at = NULL, lease_token = NULL, lease_until = NULL, \
             next_attempt_at = NULL WHERE id = $1",
        )
        .bind(uncertain_item)
        .execute(db.pool())
        .await
        .expect("park the uncertain item");
        sqlx::query(
            "UPDATE context69.task_items SET status = 'running', stage = 'processing', \
             finished_at = NULL, lease_token = NULL, lease_until = NULL WHERE id = $1",
        )
        .bind(known_item)
        .execute(db.pool())
        .await
        .expect("run the unrelated item");
        let uncertain_job = seed_uncertain_job(db, task_id, uncertain_item).await;
        let known_job = seed_known_job(
            db,
            task_id,
            known_item,
            Utc::now() + Duration::minutes(5),
            Some(Utc::now() + Duration::hours(1)),
        )
        .await;
        let known_before = sqlx::query_file!(
            "tests/sql/docling_remote_jobs/read_probe_state.sql",
            known_job
        )
        .fetch_one(db.pool())
        .await
        .expect("read the known job before");
        let known_item_before = sqlx::query_file!(
            "tests/sql/task_execution/read_item_retry_state.sql",
            known_item
        )
        .fetch_one(db.pool())
        .await
        .expect("read the known item before");

        let mock = QdrantPointsMock::start().await;
        let service =
            build_service(db, &body_root, Arc::new(FakeEmbedding::new(0)), &mock, None).await;
        let response = service
            .retry_with_ack(task_id, user_id, true)
            .await
            .expect("the unrelated known job never refuses the acknowledgement");
        assert_eq!(response.retried_items, 1);
        assert_eq!(response.task.item_ids, vec![uncertain_item]);

        let history = submission_history(db, task_id).await;
        assert_eq!(
            history.len(),
            2,
            "the uncertain row and the unrelated known row are both retained"
        );
        let uncertain_row = history
            .iter()
            .find(|row| row.0 == uncertain_job)
            .expect("the archived uncertain row exists");
        assert_eq!(uncertain_row.1, uncertain_item);
        assert_eq!(uncertain_row.2, "superseded");
        let known_row = history
            .iter()
            .find(|row| row.0 == known_job)
            .expect("the unrelated known row stays in the history read");
        assert_eq!(known_row.1, known_item);
        assert_eq!(known_row.2, "pending");
        assert!(
            !known_row.3,
            "the known job never gained a dispatch attempt"
        );
        assert_eq!(known_row.8, "running");

        let known_after = sqlx::query_file!(
            "tests/sql/docling_remote_jobs/read_probe_state.sql",
            known_job
        )
        .fetch_one(db.pool())
        .await
        .expect("read the known job after");
        assert_eq!(known_after.job_status, "pending");
        assert_eq!(known_after.remote_status.as_deref(), Some("pending"));
        assert_eq!(known_after.next_poll_at, known_before.next_poll_at);
        assert_eq!(known_after.deadline_at, known_before.deadline_at);
        assert_eq!(known_after.item_status, "running");
        assert_eq!(known_after.waiting_reason, known_before.waiting_reason);
        let known_item_after = sqlx::query_file!(
            "tests/sql/task_execution/read_item_retry_state.sql",
            known_item
        )
        .fetch_one(db.pool())
        .await
        .expect("read the known item after");
        assert_eq!(known_item_after.status, known_item_before.status);
        assert_eq!(
            known_item_after.attempt_count,
            known_item_before.attempt_count
        );
        assert_eq!(
            known_item_after.execution_failure_count,
            known_item_before.execution_failure_count
        );
        assert_eq!(known_item_after.retryable, known_item_before.retryable);
        assert_eq!(
            known_item_after.next_attempt_at,
            known_item_before.next_attempt_at
        );
    })
    .await;
}

/// Every read projection agrees on the risk flag, the diagnose view retains the
/// superseded diagnostic and never exposes identity/endpoint/secrets.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn read_projections_agree_and_the_diagnose_view_keeps_the_diagnostic() {
    let db = connect().await;
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id).await;
    let root = std::env::temp_dir().join(format!("p5b1v-projections-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).expect("storage root");
    let body_root = root.clone();
    let db = &db;
    with_scoped_teardown(db, task_id, user_id, &root, async move {
        parked_fixture(
            db,
            task_id,
            item_id,
            json!({
                "sha256": "f".repeat(64),
                "marker": "p5b1v-secret",
                "endpoint_base_url": "http://docling.invalid/v1",
            }),
        )
        .await;

        let mock = QdrantPointsMock::start().await;
        let service =
            build_service(db, &body_root, Arc::new(FakeEmbedding::new(0)), &mock, None).await;

        let batch = |task_id: Uuid| TaskBatchGetRequest {
            task_ids: vec![task_id],
        };
        let listed = |page: &context69_contracts::TaskPageResponse, task_id: Uuid| {
            page.items
                .iter()
                .find(|task| task.task_id == task_id)
                .cloned()
        };

        let get_task = service.get(task_id, user_id).await.expect("get");
        assert!(get_task.docling_submission_uncertain);
        let list_page = service
            .list(user_id, &processing_query())
            .await
            .expect("list");
        assert!(
            listed(&list_page, task_id)
                .expect("the fixture task is listed")
                .docling_submission_uncertain
        );
        let batch_page = service
            .batch_get(user_id, &batch(task_id))
            .await
            .expect("batch get");
        let batch_task = batch_page.items[0]
            .task
            .as_ref()
            .expect("the owned task is found");
        assert!(batch_task.docling_submission_uncertain);
        let uncertain = service
            .diagnose(task_id, user_id)
            .await
            .expect("diagnose uncertain");
        assert!(uncertain.task.docling_submission_uncertain);
        assert_eq!(
            uncertain.items[0]
                .docling_submission
                .as_ref()
                .expect("the item view is present")
                .state,
            TaskDoclingSubmissionState::SubmissionUncertain
        );

        service
            .retry_with_ack(task_id, user_id, true)
            .await
            .expect("the acknowledged retry is accepted");
        assert!(
            !service
                .get(task_id, user_id)
                .await
                .expect("get after archive")
                .docling_submission_uncertain
        );
        assert!(
            !listed(
                &service
                    .list(user_id, &processing_query())
                    .await
                    .expect("list after archive"),
                task_id
            )
            .expect("the fixture task stays listed")
            .docling_submission_uncertain
        );
        let archived_batch = service
            .batch_get(user_id, &batch(task_id))
            .await
            .expect("batch get after archive");
        assert!(
            !archived_batch.items[0]
                .task
                .as_ref()
                .expect("the owned task is found")
                .docling_submission_uncertain
        );
        let archived = service
            .diagnose(task_id, user_id)
            .await
            .expect("diagnose after archive");
        assert!(!archived.task.docling_submission_uncertain);
        let archived_view = archived.items[0]
            .docling_submission
            .as_ref()
            .expect("the superseded diagnostic is retained");
        assert_eq!(archived_view.state, TaskDoclingSubmissionState::Superseded);
        assert!(archived_view.finished_at.is_some());
        let serialized = serde_json::to_string(&archived).expect("diagnose serializes");
        assert!(
            !serialized.contains("p5b1v-secret"),
            "the projection never exposes frozen identity"
        );
        assert!(
            !serialized.contains("docling.invalid"),
            "the projection never exposes the endpoint"
        );

        let reclaimed = claim_item(db, item_id).await;
        let (fence, _) = fence_of(task_id, &reclaimed);
        db.begin_docling_submission(DoclingSubmissionIntent {
            fence,
            endpoint_base_url: "http://docling.invalid/v1",
            submission_identity: &json!({ "sha256": "9".repeat(64) }),
            deadline_at: Utc::now() + Duration::hours(1),
        })
        .await
        .expect("persist the new intent")
        .expect("the reopened item accepts a new intent");

        assert!(
            service
                .get(task_id, user_id)
                .await
                .expect("get after the new intent")
                .docling_submission_uncertain
        );
        assert!(
            listed(
                &service
                    .list(user_id, &processing_query())
                    .await
                    .expect("list after the new intent"),
                task_id
            )
            .expect("the fixture task stays listed")
            .docling_submission_uncertain
        );
        let live_batch = service
            .batch_get(user_id, &batch(task_id))
            .await
            .expect("batch get after the new intent");
        assert!(
            live_batch.items[0]
                .task
                .as_ref()
                .expect("the owned task is found")
                .docling_submission_uncertain
        );
        let live = service
            .diagnose(task_id, user_id)
            .await
            .expect("diagnose the live intent");
        assert_eq!(
            live.items[0]
                .docling_submission
                .as_ref()
                .expect("the item view is present")
                .state,
            TaskDoclingSubmissionState::Submitting
        );
    })
    .await;
}

/// A settled waiting uncertainty is recovered by an acknowledged batch retry:
/// the same primitive archives the diagnostic, reopens the item, links its next
/// generation, and spends no new POST or failure budget.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn batch_acknowledged_retry_recovers_a_settled_waiting_uncertainty() {
    let db = connect().await;
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let task_id = Uuid::new_v4();
    let root = std::env::temp_dir().join(format!("r1-batch-ack-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).expect("storage root");
    let body_root = root.clone();
    let db = &db;
    let task_ids = [task_id];
    with_tasks_teardown(db, &task_ids, user_id, &root, async move {
        let item_id = create_fixture_task(db, user_id, task_id, "a").await;
        let (job_id, attempt_count) = parked_fixture(
            db,
            task_id,
            item_id,
            json!({ "sha256": "b".repeat(64), "marker": "r1-batch-ack" }),
        )
        .await;
        let mock = QdrantPointsMock::start().await;
        let service =
            build_service(db, &body_root, Arc::new(FakeEmbedding::new(0)), &mock, None).await;

        let response = service
            .batch_retry(
                user_id,
                &TaskBatchRetryRequest {
                    task_ids: vec![task_id],
                    acknowledge_docling_duplicate_risk: true,
                },
            )
            .await
            .expect("an acknowledged settled waiting task is recovered in the batch");
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].outcome, TaskBatchRetryOutcome::Accepted);
        assert_eq!(response.results[0].retried_task_id, Some(task_id));

        let history = submission_history(db, task_id).await;
        assert_eq!(history.len(), 1, "the old diagnostic is retained once");
        assert_eq!(history[0].0, job_id);
        assert_eq!(history[0].2, "superseded");
        assert_eq!(history[0].8, "queued");
        let reopened = sqlx::query_file!(
            "tests/sql/task_execution/read_item_retry_state.sql",
            item_id
        )
        .fetch_one(db.pool())
        .await
        .expect("read the reopened item");
        assert_eq!(reopened.status, "queued");
        assert_eq!(
            reopened.execution_failure_count, 0,
            "a manual retry restarts the failure budget"
        );
        assert_eq!(
            reopened.attempt_count, attempt_count,
            "the claim ordinal stays monotonic"
        );

        let repeated = service
            .batch_retry(
                user_id,
                &TaskBatchRetryRequest {
                    task_ids: vec![task_id],
                    acknowledge_docling_duplicate_risk: true,
                },
            )
            .await
            .expect("a repeated acknowledged batch retry is a normal skip, not a 409");
        assert_eq!(repeated.results[0].outcome, TaskBatchRetryOutcome::Skipped);
        assert_eq!(
            repeated.results[0].reason,
            Some(TaskBatchRetryReason::IneligibleStatus)
        );
        assert_eq!(
            submission_history(db, task_id).await.len(),
            1,
            "a repeated acknowledgement creates no new diagnostic"
        );
    })
    .await;
}

/// Superseded history never refuses a repeated acknowledgement or a mixed batch
/// that also contains an actual failed peer.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn acknowledged_history_never_blocks_a_repeated_ack_or_a_failed_peer() {
    let db = connect().await;
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let history_task = Uuid::new_v4();
    let failed_peer_task = Uuid::new_v4();
    let root = std::env::temp_dir().join(format!("r1-mixed-history-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).expect("storage root");
    let body_root = root.clone();
    let db = &db;
    let task_ids = [history_task, failed_peer_task];
    with_tasks_teardown(db, &task_ids, user_id, &root, async move {
        let history_item = create_fixture_task(db, user_id, history_task, "history").await;
        let (job_id, _) = parked_fixture(
            db,
            history_task,
            history_item,
            json!({ "sha256": "d".repeat(64), "marker": "r1-mixed-history" }),
        )
        .await;
        // The failed peer is seeded only after the history item is parked, so
        // the shared claim helper never sees an unexpected claimable fixture.
        let failed_peer_item = create_fixture_task(db, user_id, failed_peer_task, "peer").await;
        fail_item_and_recompute(db, failed_peer_task, failed_peer_item).await;

        let mock = QdrantPointsMock::start().await;
        let service =
            build_service(db, &body_root, Arc::new(FakeEmbedding::new(0)), &mock, None).await;
        let first = service
            .retry_with_ack(history_task, user_id, true)
            .await
            .expect("the first acknowledgement reopens the settled item");
        assert_eq!(first.retried_items, 1);

        let repeated = service
            .retry_with_ack(history_task, user_id, true)
            .await
            .expect_err("a repeated acknowledgement has nothing left to reopen");
        let domain = context69::domain_errors::find_domain_error(&repeated).expect("typed refusal");
        assert!(
            matches!(
                domain,
                context69::domain_errors::DomainError::InvalidArgument(_)
            ),
            "a repeated acknowledgement stays a normal invalid argument: {domain:?}"
        );

        let repeated_batch = service
            .batch_retry(
                user_id,
                &TaskBatchRetryRequest {
                    task_ids: vec![history_task],
                    acknowledge_docling_duplicate_risk: true,
                },
            )
            .await
            .expect("a repeated acknowledged batch retry is a normal skip");
        assert_eq!(
            repeated_batch.results[0].outcome,
            TaskBatchRetryOutcome::Skipped
        );

        fail_item_and_recompute(db, history_task, history_item).await;
        let mixed = service
            .batch_retry(
                user_id,
                &TaskBatchRetryRequest {
                    task_ids: vec![history_task, failed_peer_task],
                    acknowledge_docling_duplicate_risk: true,
                },
            )
            .await
            .expect("superseded history never refuses a mixed acknowledged batch");
        assert_eq!(mixed.results.len(), 2);
        for result in &mixed.results {
            assert_eq!(result.outcome, TaskBatchRetryOutcome::Accepted);
        }
        let history = submission_history(db, history_task).await;
        assert_eq!(history.len(), 1, "the archived diagnostic is retained");
        assert_eq!(history[0].0, job_id);
        assert_eq!(history[0].2, "superseded");
    })
    .await;
}

/// An actual in-flight peer still refuses the whole acknowledged batch before
/// any settled peer is archived.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn an_in_flight_peer_refuses_the_whole_acknowledged_batch() {
    let db = connect().await;
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let settled_task = Uuid::new_v4();
    let live_task = Uuid::new_v4();
    let root = std::env::temp_dir().join(format!("r1-in-flight-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).expect("storage root");
    let body_root = root.clone();
    let db = &db;
    let task_ids = [settled_task, live_task];
    with_tasks_teardown(db, &task_ids, user_id, &root, async move {
        let settled_item = create_fixture_task(db, user_id, settled_task, "settled").await;
        // Park the settled item before the live peer exists, so the shared
        // claim helper never sees two claimable fixture items.
        let (settled_job, _) = parked_fixture(
            db,
            settled_task,
            settled_item,
            json!({ "sha256": "e".repeat(64), "marker": "r1-in-flight-settled" }),
        )
        .await;
        let live_item = create_fixture_task(db, user_id, live_task, "live").await;
        let claimed = claim_item(db, live_item).await;
        let (fence, _) = fence_of(live_task, &claimed);
        let live_job = db
            .begin_docling_submission(DoclingSubmissionIntent {
                fence,
                endpoint_base_url: "http://docling.invalid/v1",
                submission_identity: &json!({ "sha256": "f".repeat(64), "size_bytes": 8 }),
                deadline_at: Utc::now() + Duration::hours(1),
            })
            .await
            .expect("persist the live intent")
            .expect("the owned item accepts the live intent")
            .id;

        let mock = QdrantPointsMock::start().await;
        let service =
            build_service(db, &body_root, Arc::new(FakeEmbedding::new(0)), &mock, None).await;
        let error = service
            .batch_retry(
                user_id,
                &TaskBatchRetryRequest {
                    task_ids: vec![settled_task, live_task],
                    acknowledge_docling_duplicate_risk: true,
                },
            )
            .await
            .expect_err("an in-flight peer refuses the whole acknowledged batch");
        let domain = context69::domain_errors::find_domain_error(&error).expect("typed refusal");
        assert!(matches!(
            domain,
            context69::domain_errors::DomainError::DoclingSubmissionUncertain(_)
        ));
        assert_eq!(domain.status_code_number(), 409);
        assert_eq!(
            submission_state(db, settled_job).await.0,
            "submission_uncertain",
            "the settled peer is not archived by a refused batch"
        );
        assert_eq!(
            submission_state(db, live_job).await.0,
            "submitting",
            "the live intent is unchanged by the refusal"
        );
        let history = submission_history(db, settled_task).await;
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].2, "submission_uncertain");
    })
    .await;
}
