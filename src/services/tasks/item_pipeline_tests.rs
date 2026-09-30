//! Issue 592 regression coverage: state propagation across the collapsed
//! worker stages.
//!
//! The storage stage writes `file_id`/payload back into the worker's item
//! snapshot; the next stage must observe it. These tests run the real stage
//! handlers (`process_text`, `process_file`, `process_url`) through the real
//! `drive_item` loop, short-circuiting only the network-only
//! docling/embedding/indexing hop, then complete the item with the same DB
//! calls `runtime::run_item` uses. Skipped when `CONTEXT69_TEST_DATABASE_URL`
//! is unset, like the other DB-backed suites.

mod fixtures;
mod support;

use context69_contracts::TaskKind;
use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

use crate::db::{CreateTaskSubmissionRequest, Database, FinishTaskItemRequest};
use crate::services::tasks::item_file_processors::process_text;
use crate::services::tasks::item_processors::{ProcessResult, drive_item};

use fixtures::{cleanup, payload, seed_group, seed_user};
use support::{PIPELINE_CASE_LOCK, RealStorageRunner, build_service};

/// Runs one kind from its entry stage to terminal success and asserts the task
/// result, terminal task/item state, and `file_id`/`library_files` consistency.
async fn run_path_case(
    kind: TaskKind,
    kind_str: &str,
    payload: Value,
    entry_stage: &'static str,
    later_stage: &'static str,
) {
    let Some(url) = std::env::var("CONTEXT69_TEST_DATABASE_URL").ok() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping issue 592 {kind_str} test");
        return;
    };
    // `claim_items` is global, so hold the shared-database lock for the whole
    // case (claim through cleanup); parallel cases would otherwise steal each
    // other's rows.
    let _guard = PIPELINE_CASE_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_user(&db).await;
    let group = seed_group(&db).await;
    let (task_id, _reused, item_ids) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id: Uuid::new_v4(),
            user_id,
            group_id: Some(group.id),
            kind: kind_str,
            group_path: Some("test/issue-592"),
            source_key: None,
            payloads: std::slice::from_ref(&payload),
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: &format!("issue-592-{kind_str}-{}", Uuid::new_v4()),
        })
        .await
        .expect("create task");
    let mut item = db
        .claim_items(50)
        .await
        .expect("claim item")
        .into_iter()
        .find(|item| item.id == item_ids[0])
        .expect("claim our item");
    assert!(
        item.file_id.is_none(),
        "the payload must start without a file_id so storage creates one"
    );
    let task = db
        .get_task_internal(task_id)
        .await
        .expect("load task")
        .expect("task exists");

    let (service, storage_root) = build_service(&db).await;
    let runner = RealStorageRunner {
        service: &service,
        db: &db,
        kind,
        group: &group,
        task: &task,
        later_stage,
    };
    let result = drive_item(&mut item, entry_stage, &runner)
        .await
        .expect("drive the collapsed pipeline");

    let ProcessResult::Succeeded(Some(resource_id)) = result else {
        panic!("storage -> {later_stage} -> finalize must succeed");
    };
    let file_id = item
        .file_id
        .expect("the shared snapshot must carry storage's file_id");
    assert_eq!(resource_id, file_id.to_string());

    // Terminal completion through the same DB calls `runtime::run_item` uses.
    let finished = db
        .finish_task_item(FinishTaskItemRequest {
            task_id,
            item_id: item.id,
            status: "succeeded",
            resource_id: Some(resource_id.as_str()),
            failure_stage: None,
            error_message: None,
            retryable: true,
            lease_token: item.lease_token,
            attempt_id: item.attempt_id,
        })
        .await
        .expect("finish item");
    assert!(finished, "the lease must still be valid at completion");
    db.recompute_task(task_id).await.expect("recompute task");

    let row = sqlx::query(
        "SELECT ti.status, ti.file_id, ti.resource_id, lf.group_id \
         FROM context69.task_items ti \
         JOIN context69.library_files lf ON lf.id = ti.file_id \
         WHERE ti.id = $1",
    )
    .bind(item.id)
    .fetch_one(db.pool())
    .await
    .expect("task_items.file_id must reference a library_files row");
    assert_eq!(row.get::<String, _>("status"), "succeeded");
    assert_eq!(row.get::<Option<Uuid>, _>("file_id"), Some(file_id));
    assert_eq!(
        row.get::<Option<String>, _>("resource_id").as_deref(),
        Some(resource_id.as_str())
    );
    assert_eq!(row.get::<i64, _>("group_id"), group.id);

    let task = db
        .get_task_internal(task_id)
        .await
        .expect("reload task")
        .expect("task exists");
    assert_eq!(task.status, "succeeded");
    assert_eq!(task.succeeded_count, 1);
    assert_eq!(task.failed_count, 0);

    cleanup(&db, group.id, user_id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}

#[tokio::test]
async fn text_batch_storage_reaches_indexing_with_persisted_file() {
    run_path_case(
        TaskKind::TextBatch,
        "text_batch",
        payload("text_batch"),
        "storage",
        "indexing",
    )
    .await;
}

/// Issue 662 regression: once the storage stage persisted the text sections,
/// a re-claimed item must resume at `indexing`. Before the guard the stored
/// payload was re-parsed as `UpsertLibraryTextRequest`, which rejects the
/// `section_payload` key and looped storage retries forever.
#[tokio::test]
async fn text_batch_resumes_at_indexing_after_persisted_sections() {
    let Some(url) = std::env::var("CONTEXT69_TEST_DATABASE_URL").ok() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping issue 662 text recovery test");
        return;
    };
    let _guard = PIPELINE_CASE_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_user(&db).await;
    let group = seed_group(&db).await;
    let (task_id, _reused, item_ids) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id: Uuid::new_v4(),
            user_id,
            group_id: Some(group.id),
            kind: "text_batch",
            group_path: Some("test/issue-662"),
            source_key: None,
            payloads: std::slice::from_ref(&payload("text_batch")),
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: &format!("issue-662-{}", Uuid::new_v4()),
        })
        .await
        .expect("create task");
    let mut item = db
        .claim_items(50)
        .await
        .expect("claim item")
        .into_iter()
        .find(|item| item.id == item_ids[0])
        .expect("claim our item");
    let task = db
        .get_task_internal(task_id)
        .await
        .expect("load task")
        .expect("task exists");

    let (service, storage_root) = build_service(&db).await;

    // First storage run: real upsert plus `save_sections` persisting the
    // parsed sections into the item payload.
    let first = process_text(&service, Some(&group), &task, &mut item, "storage")
        .await
        .expect("run text storage");
    assert!(
        matches!(first, ProcessResult::Progressed { next: "indexing" }),
        "the first text storage run must advance to indexing"
    );

    // Re-claim semantics: the retry reads the persisted payload, not the
    // in-memory snapshot.
    let stored: Value =
        sqlx::query_scalar("SELECT payload FROM context69.task_items WHERE id = $1")
            .bind(item.id)
            .fetch_one(db.pool())
            .await
            .expect("load persisted item payload");
    assert!(
        stored
            .get("section_payload")
            .is_some_and(|value| !value.is_null()),
        "storage must persist the parsed sections"
    );
    let mut resumed = item.clone();
    resumed.payload = stored;
    let recovered = process_text(&service, Some(&group), &task, &mut resumed, "storage")
        .await
        .expect("resume text storage");
    assert!(
        matches!(recovered, ProcessResult::Progressed { next: "indexing" }),
        "a persisted-section item must resume at indexing, not re-parse the initial request"
    );

    cleanup(&db, group.id, user_id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}

#[tokio::test]
async fn file_batch_storage_reaches_embedding_with_persisted_file() {
    run_path_case(
        TaskKind::FileBatch,
        "file_batch",
        payload("file_batch"),
        "storage",
        "embedding",
    )
    .await;
}

#[tokio::test]
async fn url_batch_storage_reaches_embedding_with_persisted_file() {
    run_path_case(
        TaskKind::UrlBatch,
        "url_batch",
        payload("url_batch"),
        "download",
        "embedding",
    )
    .await;
}
