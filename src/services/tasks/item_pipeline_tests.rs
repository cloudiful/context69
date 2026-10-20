//! Issue 592 regression coverage: state propagation across the collapsed
//! worker stages. Issue 667 Phase 1 streamed-source coverage lives in the
//! `source_materialization` submodule.
//!
//! The storage stage writes `file_id`/payload back into the worker's item
//! snapshot; the next stage must observe it. These tests run the real stage
//! handlers (`process_text`, `process_file`, `process_url`) through the real
//! `drive_item` loop, short-circuiting only the network-only
//! docling/embedding/indexing hop, then complete the item with the same DB
//! calls `runtime::run_item` uses. Skipped when `CONTEXT69_TEST_DATABASE_URL`
//! is unset, like the other DB-backed suites.

mod fixtures;
mod source_materialization;
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
    let url = context69_test_support::accessor::database_url();
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
#[ignore = "requires configured reusable PostgreSQL"]
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
#[ignore = "requires configured reusable PostgreSQL"]
async fn text_batch_resumes_at_indexing_after_persisted_sections() {
    let url = context69_test_support::accessor::database_url();
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

    // Re-claim semantics: the retry reads the persisted payload and the
    // externalized sections, not the in-memory snapshot.
    let stored: Value =
        sqlx::query_scalar("SELECT payload FROM context69.task_items WHERE id = $1")
            .bind(item.id)
            .fetch_one(db.pool())
            .await
            .expect("load persisted item payload");
    assert!(
        stored.get("section_payload").is_none_or(Value::is_null),
        "a group-owned item externalizes its sections instead of keeping the inline copy"
    );
    assert_eq!(
        stored.get("section_payload"),
        None,
        "externalization removes the inline copy from the durable payload"
    );
    let mut resumed = item.clone();
    resumed.payload = stored;
    let resolved =
        crate::services::tasks::item_processors::resolved_section_payload(&service, &resumed)
            .await
            .expect("resolve the externalized sections")
            .expect("the artifact reference still yields the sections");
    let resolved = resolved.as_array().expect("section array");
    assert_eq!(resolved.len(), 1, "one document section is externalized");
    assert_eq!(resolved[0]["section_key"], "document");
    assert_eq!(
        resolved[0]["body_text"], "the storage stage creates the file and its sections",
        "the externalized sections keep the same content"
    );
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
#[ignore = "requires configured reusable PostgreSQL"]
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
#[ignore = "requires configured reusable PostgreSQL"]
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

/// Attach an externalized `sections` artifact to a claimed item, returning the
/// verified object id. Used by the artifact-aware reader cases.
async fn attach_sections_artifact(
    _db: &Database,
    service: &crate::services::tasks::TaskService,
    group_id: i64,
    item: &crate::db::ClaimedItem,
    sections: &Value,
) -> Uuid {
    let staged = service
        .library()
        .stage_task_item_artifact(
            group_id,
            bytes::Bytes::from(serde_json::to_vec(sections).expect("serialize sections")),
            item.lease_token,
        )
        .await
        .expect("stage sections artifact");
    assert!(
        service
            .library()
            .attach_task_item_artifact(
                item.id,
                item.lease_token,
                crate::db::TaskItemArtifactKind::Sections,
                &staged,
            )
            .await
            .expect("attach sections artifact"),
        "the running lease must attach the sections"
    );
    staged.object_id
}

fn externalized_sections() -> Value {
    serde_json::json!([{
        "section_key": "document",
        "section_label": "Issue 592",
        "title": "Issue 592",
        "summary": null,
        "body_text": "the storage stage creates the file and its sections",
        "source_uri": null,
        "external_id": null,
        "published_at": null,
        "metadata_json": {},
    }])
}

/// Issue #975 P4: once the inline sections were externalized, the task reader
/// resolves the identical JSON from the artifact and never reconverts.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn artifact_sections_resolve_to_the_identical_json_without_an_inline_copy() {
    let url = context69_test_support::accessor::database_url();
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
            group_path: Some("test/issue-975"),
            source_key: None,
            payloads: std::slice::from_ref(&payload("text_batch")),
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: &format!("issue-975-ref-only-{}", Uuid::new_v4()),
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

    // The real storage stage saves the sections inline first and then
    // externalizes them, so the durable payload no longer carries the text.
    let stored = process_text(&service, Some(&group), &task, &mut item, "storage")
        .await
        .expect("run text storage");
    assert!(matches!(
        stored,
        ProcessResult::Progressed { next: "indexing" }
    ));
    let persisted: Value =
        sqlx::query_scalar("SELECT payload FROM context69.task_items WHERE id = $1")
            .bind(item.id)
            .fetch_one(db.pool())
            .await
            .expect("load persisted payload");
    assert!(
        persisted.get("section_payload").is_none(),
        "the artifact attach removes the inline copy"
    );
    item.payload = persisted;
    let expected =
        crate::services::tasks::item_processors::resolved_section_payload(&service, &item)
            .await
            .expect("resolve artifact sections")
            .expect("the artifact yields the sections");
    assert_eq!(
        expected[0]["body_text"],
        "the storage stage creates the file and its sections"
    );

    // Re-entering `docling` with artifact-only sections must skip the remote
    // conversion: without sections this fixture has no converter and would
    // fail instead of advancing.
    let outcome = process_text(&service, Some(&group), &task, &mut item, "docling")
        .await
        .expect("run the docling stage");
    assert!(
        matches!(outcome, ProcessResult::Progressed { next } if next == "embedding"),
        "an artifact-only item must advance to embedding without a Docling submit"
    );

    cleanup(&db, group.id, user_id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}

/// A corrupt sections artifact is an error, never "no sections": reporting it
/// absent would reconvert the file and duplicate the remote work.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_corrupt_sections_artifact_errors_instead_of_reporting_no_sections() {
    let url = context69_test_support::accessor::database_url();
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
            kind: "file_batch",
            group_path: Some("test/issue-975"),
            source_key: None,
            payloads: std::slice::from_ref(&payload("file_batch")),
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: &format!("issue-975-corrupt-{}", Uuid::new_v4()),
        })
        .await
        .expect("create task");
    let item = db
        .claim_items(50)
        .await
        .expect("claim item")
        .into_iter()
        .find(|item| item.id == item_ids[0])
        .expect("claim our item");
    let (service, storage_root) = build_service(&db).await;
    let object_id =
        attach_sections_artifact(&db, &service, group.id, &item, &externalized_sections()).await;
    let object = service
        .library()
        .load_task_item_artifact(item.id, crate::db::TaskItemArtifactKind::Sections)
        .await
        .expect("load reference")
        .expect("reference exists");
    assert_eq!(object.storage_object_id, object_id);
    std::fs::write(
        storage_root.join(&object.object_key),
        b"this is not the stored json",
    )
    .expect("corrupt the stored object");

    let error = crate::services::tasks::item_processors::resolved_section_payload(&service, &item)
        .await
        .expect_err("a corrupt artifact must surface as an error");
    let message = error.to_string();
    assert!(
        message.contains("integrity check failed") || message.contains("not valid JSON"),
        "the error must name the corruption, got: {message}"
    );

    cleanup(&db, group.id, user_id).await;
    let _ = std::fs::remove_dir_all(storage_root);
    let _ = task_id;
}

/// A later progress save in the same claim must not reinsert the externalized
/// section text into the item payload.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_later_progress_save_never_reintroduces_the_externalized_sections() {
    let url = context69_test_support::accessor::database_url();
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
            group_path: Some("test/issue-975"),
            source_key: None,
            payloads: std::slice::from_ref(&payload("text_batch")),
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: &format!("issue-975-progress-{}", Uuid::new_v4()),
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

    let first = process_text(&service, Some(&group), &task, &mut item, "storage")
        .await
        .expect("run text storage");
    assert!(matches!(
        first,
        ProcessResult::Progressed { next: "indexing" }
    ));
    assert!(
        item.payload.get("section_payload").is_none(),
        "the in-memory snapshot drops the externalized sections"
    );
    let sections =
        crate::services::tasks::item_processors::resolved_section_payload(&service, &item)
            .await
            .expect("resolve sections")
            .expect("artifact yields sections");
    // The indexing stage builds its progress payload from the item snapshot;
    // an externalized snapshot therefore cannot reintroduce the text.
    let progressed = crate::services::library::payload_with_checkpoint(
        &item.payload,
        &crate::services::library::IndexingCheckpoint {
            next_batch_index: 1,
            total_batches: Some(1),
            ..Default::default()
        },
    )
    .expect("build the progress payload");
    assert!(
        db.set_task_item_payload(item.id, item.lease_token, &progressed)
            .await
            .expect("save indexing progress")
    );
    item.payload = progressed;

    let stored: Value =
        sqlx::query_scalar("SELECT payload FROM context69.task_items WHERE id = $1")
            .bind(item.id)
            .fetch_one(db.pool())
            .await
            .expect("load persisted payload");
    assert!(
        stored.get("section_payload").is_none(),
        "progress must not reinsert the externalized sections"
    );
    assert!(
        stored.get("indexing_checkpoint").is_some(),
        "the progress checkpoint itself is still saved"
    );
    let resolved =
        crate::services::tasks::item_processors::resolved_section_payload(&service, &item)
            .await
            .expect("resolve after progress")
            .expect("artifact still yields the sections");
    assert_eq!(resolved, sections);

    cleanup(&db, group.id, user_id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}

/// Issue #975 P4: a backfilled `download` artifact feeds the URL storage stage
/// instead of the (absent) inline bytes, and the resulting file reuses the same
/// content-addressed object rather than uploading the bytes again.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_backfilled_download_artifact_feeds_the_url_storage_stage_without_a_second_upload() {
    let url = context69_test_support::accessor::database_url();
    let _guard = PIPELINE_CASE_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_user(&db).await;
    let group = seed_group(&db).await;
    let mut payload = payload("url_batch");
    let content = bytes::Bytes::from_static(b"backfilled url bytes");
    let sha256 = base64_sha(&content);
    payload["download_artifact"]["sha256"] = Value::String(sha256.clone());
    payload["download_artifact"]
        .as_object_mut()
        .expect("artifact object")
        .remove("content_base64");
    let (task_id, _reused, item_ids) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id: Uuid::new_v4(),
            user_id,
            group_id: Some(group.id),
            kind: "url_batch",
            group_path: Some("test/issue-975"),
            source_key: None,
            payloads: std::slice::from_ref(&payload),
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: &format!("issue-975-url-{}", Uuid::new_v4()),
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
    let staged = service
        .library()
        .stage_task_item_artifact(group.id, content.clone(), item.lease_token)
        .await
        .expect("stage download artifact");
    assert_eq!(staged.sha256, sha256);
    assert!(
        service
            .library()
            .attach_task_item_artifact(
                item.id,
                item.lease_token,
                crate::db::TaskItemArtifactKind::Download,
                &staged,
            )
            .await
            .expect("attach download artifact")
    );

    let outcome = crate::services::tasks::item_url_processor::process_url(
        &service,
        Some(&group),
        &task,
        &mut item,
        "storage",
    )
    .await
    .expect("run url storage");
    assert!(
        matches!(outcome, ProcessResult::Progressed { .. }),
        "the storage stage must consume the artifact bytes"
    );
    let file_id = item.file_id.expect("the storage stage creates a file");
    let (file_object, size): (Option<Uuid>, i64) = sqlx::query_as(
        "SELECT storage_object_id, size_bytes FROM context69.library_files WHERE id = $1",
    )
    .bind(file_id)
    .fetch_one(db.pool())
    .await
    .expect("load the created file");
    assert_eq!(
        file_object,
        Some(staged.object_id),
        "the file reuses the artifact object instead of storing the bytes again"
    );
    assert_eq!(size, content.len() as i64);
    let objects: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM context69.library_storage_objects WHERE group_id = $1",
    )
    .bind(group.id)
    .fetch_one(db.pool())
    .await
    .expect("count group objects");
    assert_eq!(objects, 1, "no second object is uploaded");

    cleanup(&db, group.id, user_id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}

fn base64_sha(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Issue #975 P8: an existing but unusable `sections` reference is an error,
/// never "no sections". Reporting it absent would make the file stage reconvert
/// the document and duplicate the remote work.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn an_unusable_sections_reference_errors_instead_of_reconverting() {
    let url = context69_test_support::accessor::database_url();
    let _guard = PIPELINE_CASE_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_user(&db).await;
    let group = seed_group(&db).await;
    let other_group = seed_group(&db).await;
    let (task_id, _reused, item_ids) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id: Uuid::new_v4(),
            user_id,
            group_id: Some(group.id),
            kind: "file_batch",
            group_path: Some("test/issue-975"),
            source_key: None,
            payloads: std::slice::from_ref(&payload("file_batch")),
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: &format!("issue-975-unusable-{}", Uuid::new_v4()),
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
        .get_task(task_id, user_id)
        .await
        .expect("read task")
        .expect("task exists");
    let (service, storage_root) = build_service(&db).await;
    let object_id =
        attach_sections_artifact(&db, &service, group.id, &item, &externalized_sections()).await;
    let object = service
        .library()
        .load_task_item_artifact(item.id, crate::db::TaskItemArtifactKind::Sections)
        .await
        .expect("load reference")
        .expect("reference exists");
    assert_eq!(object.storage_object_id, object_id);
    // Corrupt the physical bytes so any real read would fail with an integrity
    // error; the refusal below must come from the identity check instead.
    std::fs::write(
        storage_root.join(&object.object_key),
        b"this is not the stored json",
    )
    .expect("corrupt the stored object");

    sqlx::query(
        "UPDATE context69.library_storage_objects SET storage_backend = 's3' WHERE id = $1",
    )
    .bind(object_id)
    .execute(db.pool())
    .await
    .expect("switch the registry backend");

    let error = crate::services::tasks::item_processors::resolved_section_payload(&service, &item)
        .await
        .expect_err("an unusable existing reference must be an error");
    let message = error.to_string();
    assert!(
        message.contains("inactive backend"),
        "the error must name the unusable identity, got: {message}"
    );
    assert!(
        !message.contains("integrity"),
        "the object must not have been read, got: {message}"
    );

    let persisted: Value =
        sqlx::query_scalar("SELECT payload FROM context69.task_items WHERE id = $1")
            .bind(item.id)
            .fetch_one(db.pool())
            .await
            .expect("load persisted payload");
    assert!(
        persisted.get("section_payload").is_none(),
        "the broken reference must not be replaced with inline data"
    );
    item.payload = persisted;

    // The docling stage reports the failure; it never submits a conversion. The
    // file id is only a local snapshot value: the stage resolves sections before
    // it ever looks at the file.
    item.file_id = Some(Uuid::new_v4());
    let outcome = process_text(&service, Some(&group), &task, &mut item, "docling")
        .await
        .expect("stage returns a process result");
    assert!(
        matches!(outcome, ProcessResult::Failed { .. }),
        "an unusable reference must fail the stage"
    );

    // A registry group that disagrees with the task group is refused too.
    sqlx::query(
        "UPDATE context69.library_storage_objects SET storage_backend = 'local', group_id = $2 \
         WHERE id = $1",
    )
    .bind(object_id)
    .bind(other_group.id)
    .execute(db.pool())
    .await
    .expect("switch the registry group");
    let error = crate::services::tasks::item_processors::resolved_section_payload(&service, &item)
        .await
        .expect_err("a group mismatch must be an error");
    assert!(
        error.to_string().contains("another group"),
        "the error must name the group mismatch, got: {error}"
    );

    cleanup(&db, group.id, user_id).await;
    cleanup(&db, other_group.id, user_id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}
