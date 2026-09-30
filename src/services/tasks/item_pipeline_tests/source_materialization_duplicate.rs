//! Issue 667 Phase 1: two sources with identical content share one
//! content-addressed storage object, and the staging lease is released only
//! once no item still needs it.

use uuid::Uuid;

use crate::db::Database;
use crate::services::tasks::item_processors::ProcessResult;
use crate::services::tasks::item_url_processor::process_url;

use super::super::fixtures::{cleanup, seed_group, seed_user};
use super::super::support::{PIPELINE_CASE_LOCK, build_service};
use super::{claim_item, create_url_task, sha256_hex, staged_url_payload, uploaded_file};

/// Issue 667 Phase 1: two sources with identical content share one
/// content-addressed storage object, and the staging lease is released only
/// once no item still needs it.
#[tokio::test]
async fn duplicate_staged_contents_share_one_content_addressed_object() {
    let Some(database_url) = std::env::var("CONTEXT69_TEST_DATABASE_URL").ok() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping issue 667 duplicate test");
        return;
    };
    let _guard = PIPELINE_CASE_LOCK.lock().await;
    let db = Database::connect(&database_url)
        .await
        .expect("connect test database");
    let user_id = seed_user(&db).await;
    let group = seed_group(&db).await;
    let (service, storage_root) = build_service(&db).await;

    let content = b"issue 667 duplicate staged body";
    let sha256 = sha256_hex(content);
    let first_object = service
        .library()
        .stage_file_for_task_input(group.id, uploaded_file(&sha256, content, "issue-667-dup-a"))
        .await
        .expect("stage the first input object");
    let second_object = service
        .library()
        .stage_file_for_task_input(group.id, uploaded_file(&sha256, content, "issue-667-dup-b"))
        .await
        .expect("stage the duplicate input object");
    assert_eq!(
        first_object, second_object,
        "same group and digest must reuse the catalog object"
    );

    let payloads = [
        staged_url_payload(&sha256, "issue-667-dup-a"),
        staged_url_payload(&sha256, "issue-667-dup-b"),
    ];
    let inputs: [Option<Uuid>; 2] = [Some(first_object), Some(second_object)];
    let mut files = Vec::new();
    for (index, payload) in payloads.iter().enumerate() {
        // One item per task: the dispatcher claims a parent's items strictly in
        // ordinal order, so a second item of the same task cannot be claimed
        // while the first is still running.
        let (task_id, _reused, item_ids) = create_url_task(
            &db,
            user_id,
            group.id,
            std::slice::from_ref(payload),
            &[inputs[index]],
        )
        .await;
        let task = db
            .get_task_internal(task_id)
            .await
            .expect("load task")
            .expect("task exists");
        let mut item = claim_item(&db, item_ids[0]).await;
        let stored = process_url(&service, Some(&group), &task, &mut item, "storage")
            .await
            .expect("run the storage stage");
        assert!(matches!(
            stored,
            ProcessResult::Progressed { next: "embedding" }
        ));
        files.push(item.file_id.expect("storage sets file_id"));
    }

    let object_rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM context69.library_storage_objects WHERE group_id = $1 AND sha256 = $2",
    )
    .bind(group.id)
    .bind(&sha256)
    .fetch_one(db.pool())
    .await
    .expect("count storage objects");
    assert_eq!(object_rows, 1, "duplicate content keeps one storage object");

    for file_id in &files {
        let object_id: Option<Uuid> = sqlx::query_scalar(
            "SELECT storage_object_id FROM context69.library_files WHERE id = $1",
        )
        .bind(file_id)
        .fetch_one(db.pool())
        .await
        .expect("load file storage object");
        assert_eq!(object_id, Some(first_object));
    }

    let lease: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
        "SELECT staging_lease_until FROM context69.library_storage_objects WHERE id = $1",
    )
    .bind(first_object)
    .fetch_one(db.pool())
    .await
    .expect("load staging lease");
    assert_eq!(
        lease, None,
        "the lease is released once no item references the staged object"
    );

    cleanup(&db, group.id, user_id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}
