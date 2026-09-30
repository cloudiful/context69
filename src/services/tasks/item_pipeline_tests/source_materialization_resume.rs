//! Issue 667 Phase 1: a re-claim resumes from a committed staged object
//! reference instead of re-downloading the URL.

use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

use crate::db::{Database, FinishTaskItemRequest};
use crate::services::tasks::item_processors::ProcessResult;
use crate::services::tasks::item_url_processor::process_url;

use super::super::fixtures::{cleanup, seed_group, seed_user};
use super::super::support::{PIPELINE_CASE_LOCK, build_service};
use super::{claim_item, create_url_task, sha256_hex, staged_url_payload, uploaded_file};

/// Issue 667 Phase 1: after the download stage streams the source into a
/// staged object and commits the reference, a re-claim resumes from
/// `input_storage_object_id` — no re-download, no bytes in the task payload —
/// and the storage stage consumes the staged bytes through the existing
/// FileBatch staged-object path.
#[tokio::test]
async fn url_batch_resumes_from_a_committed_staged_input_object() {
    let Some(database_url) = std::env::var("CONTEXT69_TEST_DATABASE_URL").ok() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping issue 667 resume test");
        return;
    };
    let _guard = PIPELINE_CASE_LOCK.lock().await;
    let db = Database::connect(&database_url)
        .await
        .expect("connect test database");
    let user_id = seed_user(&db).await;
    let group = seed_group(&db).await;
    let (service, storage_root) = build_service(&db).await;

    let content = b"issue 667 staged url body";
    let sha256 = sha256_hex(content);
    let object_id = service
        .library()
        .stage_file_for_task_input(
            group.id,
            uploaded_file(&sha256, content, "issue-667-resume"),
        )
        .await
        .expect("stage the streamed input object");
    let payload = staged_url_payload(&sha256, "issue-667-resume");
    let (task_id, _reused, item_ids) = create_url_task(
        &db,
        user_id,
        group.id,
        std::slice::from_ref(&payload),
        &[Some(object_id)],
    )
    .await;
    let mut item = claim_item(&db, item_ids[0]).await;
    assert_eq!(
        item.input_storage_object_id,
        Some(object_id),
        "the submission must carry the staged reference"
    );
    let task = db
        .get_task_internal(task_id)
        .await
        .expect("load task")
        .expect("task exists");

    // Crash/retry boundary: the committed artifact metadata plus the object
    // reference make the download stage a no-op instead of a second fetch.
    let resumed = process_url(&service, Some(&group), &task, &mut item, "download")
        .await
        .expect("resume the download stage");
    assert!(matches!(
        resumed,
        ProcessResult::Progressed { next: "storage" }
    ));

    let stored = process_url(&service, Some(&group), &task, &mut item, "storage")
        .await
        .expect("run the storage stage");
    assert!(
        matches!(stored, ProcessResult::Progressed { next: "embedding" }),
        "a staged text source advances to the embedding stage"
    );

    let file_id = item.file_id.expect("the storage stage must set file_id");
    let row = sqlx::query(
        "SELECT ti.input_storage_object_id, ti.payload, lf.sha256, lf.storage_object_id, \
         lso.object_key, lso.staging_lease_until \
         FROM context69.task_items ti \
         JOIN context69.library_files lf ON lf.id = ti.file_id \
         JOIN context69.library_storage_objects lso ON lso.id = lf.storage_object_id \
         WHERE ti.id = $1",
    )
    .bind(item.id)
    .fetch_one(db.pool())
    .await
    .expect("load the staged item state");
    assert_eq!(
        row.get::<Option<Uuid>, _>("input_storage_object_id"),
        None,
        "linking the file clears the staged reference"
    );
    assert_eq!(row.get::<String, _>("sha256"), sha256);
    assert_eq!(
        row.get::<Option<Uuid>, _>("storage_object_id"),
        Some(object_id),
        "the file must keep the staged storage object"
    );
    assert_eq!(
        row.get::<String, _>("object_key"),
        format!("objects/{}/{sha256}", group.id),
        "the catalog only ever records the content-addressed key"
    );
    assert_eq!(
        row.get::<Option<chrono::DateTime<chrono::Utc>>, _>("staging_lease_until"),
        None,
        "linking the file releases the staging lease"
    );
    let stored_payload: Value = row.get("payload");
    assert!(
        stored_payload.get("download_artifact").is_none(),
        "a completed item drops the artifact metadata"
    );

    let finished = db
        .finish_task_item(FinishTaskItemRequest {
            task_id,
            item_id: item.id,
            status: "succeeded",
            resource_id: Some(file_id.to_string().as_str()),
            failure_stage: None,
            error_message: None,
            retryable: true,
            lease_token: item.lease_token,
            attempt_id: item.attempt_id,
        })
        .await
        .expect("finish item");
    assert!(finished);

    cleanup(&db, group.id, user_id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}
