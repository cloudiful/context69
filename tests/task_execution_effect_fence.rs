//! The captured task grant of every task-owned business write (issue 1020 P3).
//!
//! A task-owned write is authorized by the grant the item holds at commit time,
//! not by the state a worker read earlier. These cases pin that contract on the
//! real business rows: a crashed segment refuses every file/content/artifact
//! write and leaves the previously committed rows untouched, a live owner
//! commits and can roll back, and a capture from an earlier generation is
//! refused after the item moved on.

use context69::db::{CreateTaskSubmissionRequest, Database};
use context69::library_store::{TaskWriteDenied, TaskWriteFence};
use serde_json::json;
use uuid::Uuid;

static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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

async fn seed_user(db: &Database) -> i64 {
    sqlx::query_file_scalar!(
        "tests/sql/task_execution/seed_user.sql",
        format!("effect-fence-{}", Uuid::new_v4()),
        "Effect Fence Test",
        "unused"
    )
    .fetch_one(db.pool())
    .await
    .expect("seed user")
}

async fn seed_group(db: &Database) -> i64 {
    sqlx::query_file_scalar!(
        "tests/sql/task_execution/seed_group.sql",
        format!("effect-fence-{}", Uuid::new_v4()),
        "Effect Fence Group",
        format!("test/effect-fence-{}", Uuid::new_v4())
    )
    .fetch_one(db.pool())
    .await
    .expect("seed group")
}

async fn seed_task(db: &Database, user_id: i64, group_id: i64) -> (Uuid, Uuid) {
    sqlx::query_file!(
        "tests/sql/task_execution/seed_group_membership.sql",
        group_id,
        user_id
    )
    .execute(db.pool())
    .await
    .expect("seed group membership");
    let task_id = Uuid::new_v4();
    let (_, _, items) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id,
            user_id,
            group_id: Some(group_id),
            kind: "text_batch",
            group_path: Some("test/effect-fence"),
            source_key: None,
            payloads: &[json!({"external_id": "a"})],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: &format!("effect-fence-{}", Uuid::new_v4()),
        })
        .await
        .expect("create task");
    (task_id, items[0])
}

async fn claim(db: &Database, item_id: Uuid) -> context69::db::ClaimedItem {
    db.claim_items_fast(50)
        .await
        .expect("claim the item")
        .into_iter()
        .find(|item| item.id == item_id)
        .expect("the item is claimed")
}

/// Crashes the worker that holds the item: the authoritative library attempt and
/// its domain mirror expire together.
async fn crash(db: &Database, item: &context69::db::ClaimedItem) {
    let execution = item
        .execution_context()
        .expect("the claimed item carries its captured execution");
    sqlx::query_file!(
        "tests/sql/task_execution/expire_library_attempt_by_id.sql",
        execution.attempt_id()
    )
    .execute(db.pool())
    .await
    .expect("expire the library attempt");
    sqlx::query_file!("tests/sql/task_execution/expire_item_lease.sql", item.id)
        .execute(db.pool())
        .await
        .expect("expire the domain mirror");
}

async fn file_count(db: &Database, group_id: i64) -> i64 {
    sqlx::query_file_scalar!("tests/sql/task_execution/count_group_files.sql", group_id)
        .fetch_one(db.pool())
        .await
        .expect("count files")
}

async fn item_payload(db: &Database, item_id: Uuid) -> serde_json::Value {
    sqlx::query_file_scalar!("tests/sql/task_execution/read_item_payload.sql", item_id)
        .fetch_one(db.pool())
        .await
        .expect("read the item payload")
}

async fn cleanup(db: &Database, task_id: Uuid, user_id: i64, group_id: i64) {
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
    sqlx::query_file!("tests/sql/task_execution/delete_group_files.sql", group_id)
        .execute(db.pool())
        .await
        .expect("delete files");
    sqlx::query_file!("tests/sql/task_execution/delete_group.sql", group_id)
        .execute(db.pool())
        .await
        .expect("delete group");
    sqlx::query_file!("tests/sql/task_execution/delete_user.sql", user_id)
        .execute(db.pool())
        .await
        .expect("delete user");
}

/// The fence of an earlier generation: the item belongs to a newer segment.
fn stale_generation(fence: TaskWriteFence) -> TaskWriteFence {
    TaskWriteFence::new(
        fence.item_id(),
        fence.token(),
        fence.execution_task_id(),
        fence.generation() + 1,
    )
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_live_owner_commits_and_rolls_back_a_file_write() {
    let db = connect().await;
    let _guard = TEST_LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let group_id = seed_group(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id, group_id).await;
    let item = claim(&db, item_id).await;
    let store = context69::library_store::LibraryStore::new(db.clone());
    let fence = TaskWriteFence::from_claimed_item(&item).expect("the captured grant");

    let file = context69::library_store::NewLibraryFile {
        id: Uuid::new_v4(),
        folder_id: None,
        external_id: Some("live-owner".to_string()),
        filename: "live-owner.txt".to_string(),
        media_type: "text/plain".to_string(),
        size_bytes: 4,
        sha256: "0".repeat(64),
        storage_rel_path: "objects/test/live-owner".to_string(),
        storage_object_id: None,
        delete_source_after_processing: false,
    };
    assert!(
        store
            .create_file_in_project_fenced(fence, group_id, &file)
            .await
            .expect("fenced create")
            .is_some(),
        "a live grant commits the file"
    );
    assert_eq!(file_count(&db, group_id).await, 1);

    // A write that rolled back leaves no row: the guard and the business row
    // commit or roll back together.
    let rolled_back = context69::library_store::NewLibraryFile {
        id: Uuid::new_v4(),
        external_id: Some("rolled-back".to_string()),
        filename: "rolled-back.txt".to_string(),
        ..file.clone()
    };
    let mut write = store
        .begin_task_write(fence)
        .await
        .expect("a live grant opens the write");
    sqlx::query_file!(
        "tests/sql/task_execution/seed_pending_file.sql",
        rolled_back.id,
        group_id,
        &rolled_back.filename
    )
    .execute(write.connection())
    .await
    .expect("business row inside the write");
    write.rollback().await;
    assert_eq!(
        file_count(&db, group_id).await,
        1,
        "a rolled-back write leaves only the committed file"
    );

    cleanup(&db, task_id, user_id, group_id).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_crashed_segment_cannot_create_or_replace_a_file() {
    let db = connect().await;
    let _guard = TEST_LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let group_id = seed_group(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id, group_id).await;
    let item = claim(&db, item_id).await;
    let store = context69::library_store::LibraryStore::new(db.clone());
    let fence = TaskWriteFence::from_claimed_item(&item).expect("the captured grant");

    let committed = context69::library_store::NewLibraryFile {
        id: Uuid::new_v4(),
        folder_id: None,
        external_id: Some("before-crash".to_string()),
        filename: "before-crash.txt".to_string(),
        media_type: "text/plain".to_string(),
        size_bytes: 7,
        sha256: "1".repeat(64),
        storage_rel_path: "objects/test/before-crash".to_string(),
        storage_object_id: None,
        delete_source_after_processing: false,
    };
    store
        .create_file_in_project_fenced(fence, group_id, &committed)
        .await
        .expect("commit before the crash")
        .expect("the live grant commits");

    crash(&db, &item).await;

    let late = context69::library_store::NewLibraryFile {
        id: Uuid::new_v4(),
        external_id: Some("after-crash".to_string()),
        filename: "after-crash.txt".to_string(),
        ..committed.clone()
    };
    assert!(
        store
            .create_file_in_project_fenced(fence, group_id, &late)
            .await
            .expect("a refused create is not an error")
            .is_none(),
        "a crashed segment cannot create the file"
    );
    assert!(
        store
            .update_file_content_in_project_fenced(
                fence,
                group_id,
                committed.id,
                &context69::library_store::UpdateLibraryFileContent {
                    folder_id: None,
                    external_id: Some("before-crash".to_string()),
                    filename: "replaced.txt".to_string(),
                    media_type: "text/plain".to_string(),
                    size_bytes: 9,
                    sha256: "2".repeat(64),
                    storage_rel_path: "objects/test/replaced".to_string(),
                    storage_object_id: None,
                },
            )
            .await
            .expect("a refused update is not an error")
            .is_none(),
        "a crashed segment cannot replace the content"
    );
    assert!(
        !store
            .delete_file_in_project_fenced(fence, group_id, committed.id)
            .await
            .expect("a refused delete is not an error"),
        "a crashed segment cannot delete the file"
    );

    assert_eq!(
        file_count(&db, group_id).await,
        1,
        "only the file committed before the crash exists"
    );
    let file = sqlx::query_file!(
        "tests/sql/task_execution/read_file_name_and_sha.sql",
        committed.id
    )
    .fetch_one(db.pool())
    .await
    .expect("read the committed file");
    assert_eq!(file.filename, "before-crash.txt");
    assert_eq!(file.sha256, "1".repeat(64));

    cleanup(&db, task_id, user_id, group_id).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_stale_generation_cannot_attach_an_artifact() {
    let db = connect().await;
    let _guard = TEST_LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let group_id = seed_group(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id, group_id).await;
    let item = claim(&db, item_id).await;
    let store = context69::library_store::LibraryStore::new(db.clone());
    let fence = TaskWriteFence::from_claimed_item(&item).expect("the captured grant");
    let stale = stale_generation(fence);

    let refused = store.begin_task_write(stale).await;
    let denied = match refused {
        Ok(_) => panic!("a stale generation cannot open a write"),
        Err(error) => error,
    };
    assert!(
        denied.downcast_ref::<TaskWriteDenied>().is_some(),
        "the refusal is an ownership loss"
    );

    // The live grant still writes: only the stale capture is refused.
    let file = context69::library_store::NewLibraryFile {
        id: Uuid::new_v4(),
        folder_id: None,
        external_id: Some("live".to_string()),
        filename: "live.txt".to_string(),
        media_type: "text/plain".to_string(),
        size_bytes: 1,
        sha256: "3".repeat(64),
        storage_rel_path: "objects/test/live".to_string(),
        storage_object_id: None,
        delete_source_after_processing: false,
    };
    assert!(
        store
            .create_file_in_project_fenced(fence, group_id, &file)
            .await
            .expect("the live grant commits")
            .is_some()
    );

    // The payload checkpoint keeps its P2 fence: a crashed segment cannot
    // overwrite the checkpoint either.
    crash(&db, &item).await;
    let checkpoint = json!({"indexing_checkpoint": {"next_batch_index": 3}});
    assert!(
        !db.set_task_item_payload(item.id, fence.token(), &checkpoint)
            .await
            .expect("a refused checkpoint is not an error"),
        "a crashed segment cannot advance the checkpoint"
    );
    assert_ne!(
        item_payload(&db, item.id).await,
        checkpoint,
        "the checkpoint of the crashed segment is unchanged"
    );

    cleanup(&db, task_id, user_id, group_id).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_cancelled_item_cannot_write_business_rows() {
    let db = connect().await;
    let _guard = TEST_LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let group_id = seed_group(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id, group_id).await;
    let item = claim(&db, item_id).await;
    let store = context69::library_store::LibraryStore::new(db.clone());
    let fence = TaskWriteFence::from_claimed_item(&item).expect("the captured grant");

    assert!(
        db.cancel_task(task_id, user_id)
            .await
            .expect("cancel the task"),
        "the cancel must move the task"
    );

    let late = context69::library_store::NewLibraryFile {
        id: Uuid::new_v4(),
        folder_id: None,
        external_id: Some("cancelled".to_string()),
        filename: "cancelled.txt".to_string(),
        media_type: "text/plain".to_string(),
        size_bytes: 1,
        sha256: "4".repeat(64),
        storage_rel_path: "objects/test/cancelled".to_string(),
        storage_object_id: None,
        delete_source_after_processing: false,
    };
    assert!(
        store
            .create_file_in_project_fenced(fence, group_id, &late)
            .await
            .expect("a refused create is not an error")
            .is_none(),
        "a cancelled item cannot create a file after the cancel"
    );
    assert_eq!(
        file_count(&db, group_id).await,
        0,
        "the cancelled item leaves no file behind"
    );

    cleanup(&db, task_id, user_id, group_id).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_staged_artifact_of_a_refused_grant_is_never_referenced() {
    let db = connect().await;
    let _guard = TEST_LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let group_id = seed_group(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id, group_id).await;
    let item = claim(&db, item_id).await;
    let store = context69::library_store::LibraryStore::new(db.clone());
    let fence = TaskWriteFence::from_claimed_item(&item).expect("the captured grant");
    let _ = fence;

    // A staged-but-unreferenced object stands in for the bytes uploaded before
    // the grant was lost.
    let object_id = Uuid::new_v4();
    sqlx::query_file!(
        "tests/sql/task_execution/seed_storage_object.sql",
        object_id,
        group_id,
        "5".repeat(64),
        format!("objects/{group_id}/staged-artifact")
    )
    .execute(db.pool())
    .await
    .expect("seed a staged object");

    crash(&db, &item).await;
    let refused = store.begin_task_write(fence).await;
    assert!(
        matches!(refused, Err(ref error) if error.downcast_ref::<TaskWriteDenied>().is_some()),
        "the crashed segment cannot open the write that would carry the reference"
    );
    let lease: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_file_scalar!(
        "tests/sql/task_execution/read_storage_staging_lease.sql",
        object_id
    )
    .fetch_one(db.pool())
    .await
    .expect("read the staging lease");
    assert!(
        lease.is_some(),
        "an object no live grant referenced keeps its staging lease"
    );

    cleanup(&db, task_id, user_id, group_id).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_checkpoint_of_a_crashed_segment_is_refused() {
    let db = connect().await;
    let _guard = TEST_LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let group_id = seed_group(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id, group_id).await;
    let item = claim(&db, item_id).await;
    let fence = TaskWriteFence::from_claimed_item(&item).expect("the captured grant");

    let checkpoint = json!({"indexing_checkpoint": {"next_batch_index": 2}});
    assert!(
        context69::library_store::write_task_item_payload(&db, fence, &checkpoint)
            .await
            .expect("a live grant writes the checkpoint"),
        "the live grant commits the checkpoint"
    );
    let committed = item_payload(&db, item_id).await;
    assert_eq!(committed, checkpoint);

    crash(&db, &item).await;

    let late = json!({"indexing_checkpoint": {"next_batch_index": 9}});
    assert!(
        !context69::library_store::write_task_item_payload(&db, fence, &late)
            .await
            .expect("a refused checkpoint is not an error"),
        "a crashed segment cannot advance the checkpoint"
    );
    assert_eq!(
        item_payload(&db, item_id).await,
        committed,
        "the committed checkpoint is preserved"
    );

    // A superseded generation is refused the same way.
    let next = claim(&db, item_id).await;
    let next_fence = TaskWriteFence::from_claimed_item(&next).expect("the next grant");
    let stale = stale_generation(next_fence);
    assert!(
        !context69::library_store::write_task_item_payload(&db, stale, &late)
            .await
            .expect("a stale checkpoint is not an error"),
        "an older generation cannot overwrite the checkpoint of its successor"
    );

    cleanup(&db, task_id, user_id, group_id).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_document_delete_under_a_crashed_grant_deletes_nothing() {
    let db = connect().await;
    let _guard = TEST_LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let group_id = seed_group(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id, group_id).await;
    let item = claim(&db, item_id).await;
    let fence = TaskWriteFence::from_claimed_item(&item).expect("the captured grant");
    let store = context69::library_store::LibraryStore::new(db.clone());

    let file = context69::library_store::NewLibraryFile {
        id: Uuid::new_v4(),
        folder_id: None,
        external_id: Some("doc-delete".to_string()),
        filename: "doc-delete.txt".to_string(),
        media_type: "text/plain".to_string(),
        size_bytes: 3,
        sha256: "6".repeat(64),
        storage_rel_path: "objects/test/doc-delete".to_string(),
        storage_object_id: None,
        delete_source_after_processing: false,
    };
    store
        .create_file_in_project_fenced(fence, group_id, &file)
        .await
        .expect("commit the file")
        .expect("the live grant commits");

    crash(&db, &item).await;

    assert!(
        !store
            .delete_file_in_project_fenced(fence, group_id, file.id)
            .await
            .expect("a refused delete is not an error"),
        "a crashed segment cannot delete the document's file"
    );
    assert_eq!(
        file_count(&db, group_id).await,
        1,
        "the file of the cancelled segment survives"
    );

    cleanup(&db, task_id, user_id, group_id).await;
}

async fn file_status(db: &Database, file_id: Uuid) -> String {
    sqlx::query_file_scalar!(
        "tests/sql/task_execution/read_file_ingest_status.sql",
        file_id
    )
    .fetch_one(db.pool())
    .await
    .expect("read the file status")
}

async fn document_count(db: &Database, file_id: Uuid) -> i64 {
    sqlx::query_file_scalar!(
        "tests/sql/task_execution/count_file_documents.sql",
        file_id.to_string()
    )
    .fetch_one(db.pool())
    .await
    .expect("count documents")
}

/// The status projection and the cleanup of a partial ingest are task-owned
/// writes, so a crashed segment may neither mark a file nor delete the
/// documents of a file that its successor already owns.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_status_projection_and_a_cleanup_of_a_crashed_segment_change_nothing() {
    let db = connect().await;
    let _guard = TEST_LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let group_id = seed_group(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id, group_id).await;
    let item = claim(&db, item_id).await;
    let fence = TaskWriteFence::from_claimed_item(&item).expect("the captured grant");
    let store = context69::library_store::LibraryStore::new(db.clone());

    let live_file = context69::library_store::NewLibraryFile {
        id: Uuid::new_v4(),
        folder_id: None,
        external_id: Some("live-status".to_string()),
        filename: "live-status.txt".to_string(),
        media_type: "text/plain".to_string(),
        size_bytes: 3,
        sha256: "7".repeat(64),
        storage_rel_path: "objects/test/live-status".to_string(),
        storage_object_id: None,
        delete_source_after_processing: false,
    };
    store
        .create_file_in_project(group_id, &live_file)
        .await
        .expect("the live owner creates the file");
    assert!(
        store
            .update_file_status_fenced(
                fence,
                live_file.id,
                context69::contracts::LibraryIngestStatus::Succeeded,
                None,
                true,
            )
            .await
            .expect("the live grant writes the projection")
            .is_some(),
        "a live grant commits the status projection"
    );
    assert_eq!(file_status(&db, live_file.id).await, "succeeded");

    let crashed_file = context69::library_store::NewLibraryFile {
        id: Uuid::new_v4(),
        folder_id: None,
        external_id: Some("crashed-status".to_string()),
        filename: "crashed-status.txt".to_string(),
        media_type: "text/plain".to_string(),
        size_bytes: 3,
        sha256: "8".repeat(64),
        storage_rel_path: "objects/test/crashed-status".to_string(),
        storage_object_id: None,
        delete_source_after_processing: false,
    };
    store
        .create_file_in_project(group_id, &crashed_file)
        .await
        .expect("seed the file of the crashed segment");
    sqlx::query_file!(
        "tests/sql/task_execution/insert_file_document.sql",
        group_id,
        "library-file",
        format!("crashed-status-{}", crashed_file.id),
        "crashed status",
        "context69://library/files/crashed",
        json!({"library_file_id": crashed_file.id.to_string()}),
        "hash"
    )
    .execute(db.pool())
    .await
    .expect("seed the document of the file");
    assert_eq!(document_count(&db, crashed_file.id).await, 1);

    crash(&db, &item).await;

    assert!(
        store
            .update_file_status_fenced(
                fence,
                crashed_file.id,
                context69::contracts::LibraryIngestStatus::Succeeded,
                None,
                true,
            )
            .await
            .expect("a refused projection is not an error")
            .is_none(),
        "a crashed segment projects no status"
    );
    assert_eq!(
        file_status(&db, crashed_file.id).await,
        "failed",
        "the status of the file is preserved"
    );
    assert!(
        !store
            .delete_documents_for_library_file_fenced(fence, crashed_file.id)
            .await
            .expect("a refused cleanup is not an error"),
        "a crashed segment cleans up no document"
    );
    assert_eq!(
        document_count(&db, crashed_file.id).await,
        1,
        "the documents of the file survive the crashed segment"
    );

    sqlx::query_file!(
        "tests/sql/task_execution/delete_file_documents.sql",
        crashed_file.id.to_string()
    )
    .execute(db.pool())
    .await
    .expect("delete the seeded document");

    cleanup(&db, task_id, user_id, group_id).await;
}

#[cfg(feature = "integration-test-helpers")]
#[path = "task_execution_effect_fence/whole_ingest.rs"]
mod whole_ingest;

#[cfg(feature = "integration-test-helpers")]
mod ingest_content {
    use super::*;
    use context69::contracts::{LibraryIngestStatus, Visibility};
    use context69::db::NewMetadataIndex;
    use context69::domain::{ChunkPayload, DocumentChunk, LibraryFileDocumentRecord};
    use context69::library_store::{LibraryStore, TaskWriteFence};
    use serde_json::json;

    /// The document payload of one library file.
    fn content_payload(file_id: Uuid, group_id: i64, record_hash: &str) -> ChunkPayload {
        ChunkPayload {
            chunk_id: Uuid::new_v4(),
            document_id: 0,
            group_id,
            group_key: "p3c-content".to_string(),
            group_path: "/test/p3c-content".to_string(),
            visibility: Visibility::Public,
            source_key: "file_library".to_string(),
            external_id: "p3c-content".to_string(),
            title: "p3c content".to_string(),
            summary: None,
            source_uri: format!("context69://library/files/{file_id}"),
            published_at: None,
            updated_at_source: chrono::Utc::now(),
            record_hash: record_hash.to_string(),
            chunk_index: 0,
            chunk_text: "p3c content body".to_string(),
            metadata_json: json!({
                "library_file_id": file_id.to_string(),
                "library_section_key": "document",
                "library_section_label": "Document",
                "library_filename": "p3c-content.txt",
                "library_media_type": "text/plain",
                "section_score": 7
            }),
            content_locale: "original".to_string(),
            source_locale: None,
            translation_provider: None,
        }
    }

    fn content_chunks(document_id: i64, count: usize) -> Vec<DocumentChunk> {
        (0..count)
            .map(|index| DocumentChunk {
                id: Uuid::new_v4(),
                document_id,
                chunk_index: index as i32,
                text: format!("p3c chunk {index}"),
                record_hash: "p3c-record-hash".to_string(),
            })
            .collect()
    }

    /// The document, its version and its metadata values, in one guarded write.
    async fn write_document(
        db: &Database,
        fence: TaskWriteFence,
        payload: &ChunkPayload,
    ) -> Result<bool, String> {
        let mut write = match context69::library_store::begin_task_write(db, fence).await {
            Ok(write) => write,
            Err(error)
                if error
                    .downcast_ref::<context69::library_store::TaskWriteDenied>()
                    .is_some() =>
            {
                return Ok(false);
            }
            Err(error) => return Err(error.to_string()),
        };
        db.upsert_document_in(write.connection(), payload)
            .await
            .map_err(|error| error.to_string())?;
        write.commit().await.map_err(|error| error.to_string())
    }

    /// The chunks of one document, in one guarded write.
    async fn write_chunks(
        db: &Database,
        fence: TaskWriteFence,
        document_id: i64,
        record_hash: &str,
        chunks: &[DocumentChunk],
    ) -> Result<bool, String> {
        let mut write = match context69::library_store::begin_task_write(db, fence).await {
            Ok(write) => write,
            Err(error)
                if error
                    .downcast_ref::<context69::library_store::TaskWriteDenied>()
                    .is_some() =>
            {
                return Ok(false);
            }
            Err(error) => return Err(error.to_string()),
        };
        db.insert_document_chunks_in(write.connection(), document_id, record_hash, chunks)
            .await
            .map_err(|error| error.to_string())?;
        write.commit().await.map_err(|error| error.to_string())
    }

    async fn document_count(db: &Database, file_id: Uuid) -> i64 {
        sqlx::query_file_scalar!(
            "tests/sql/task_execution/count_file_documents.sql",
            file_id.to_string()
        )
        .fetch_one(db.pool())
        .await
        .expect("count the documents")
    }

    async fn version_count(db: &Database, file_id: Uuid) -> i64 {
        sqlx::query_file_scalar!(
            "tests/sql/task_execution/count_file_document_versions.sql",
            file_id.to_string()
        )
        .fetch_one(db.pool())
        .await
        .expect("count the versions")
    }

    async fn metadata_value_count(db: &Database, file_id: Uuid) -> i64 {
        sqlx::query_file_scalar!(
            "tests/sql/task_execution/count_file_document_metadata_values.sql",
            file_id.to_string()
        )
        .fetch_one(db.pool())
        .await
        .expect("count the metadata values")
    }

    async fn chunk_count(db: &Database, file_id: Uuid) -> i64 {
        sqlx::query_file_scalar!(
            "tests/sql/task_execution/count_file_document_chunks.sql",
            file_id.to_string()
        )
        .fetch_one(db.pool())
        .await
        .expect("count the chunks")
    }

    async fn link_count(db: &Database, file_id: Uuid) -> i64 {
        sqlx::query_file_scalar!(
            "tests/sql/task_execution/count_file_document_links.sql",
            file_id
        )
        .fetch_one(db.pool())
        .await
        .expect("count the links")
    }

    async fn file_status(db: &Database, file_id: Uuid) -> String {
        sqlx::query_file_scalar!(
            "tests/sql/task_execution/read_file_ingest_status.sql",
            file_id
        )
        .fetch_one(db.pool())
        .await
        .expect("read the file status")
    }

    /// The rows one ingest produced, read back from the database.
    #[derive(Debug, PartialEq, Eq)]
    struct ContentRows {
        documents: i64,
        versions: i64,
        metadata_values: i64,
        chunks: i64,
        links: i64,
        status: String,
    }

    async fn content_rows(db: &Database, file_id: Uuid) -> ContentRows {
        ContentRows {
            documents: document_count(db, file_id).await,
            versions: version_count(db, file_id).await,
            metadata_values: metadata_value_count(db, file_id).await,
            chunks: chunk_count(db, file_id).await,
            links: link_count(db, file_id).await,
            status: file_status(db, file_id).await,
        }
    }

    async fn seed_library_file(db: &Database, group_id: i64) -> Uuid {
        let file_id = Uuid::new_v4();
        sqlx::query_file!(
            "tests/sql/task_execution/seed_failed_file.sql",
            file_id,
            group_id,
            "p3c-content",
            "p3c-content.txt",
            16_i64,
            "a".repeat(64),
            format!("objects/{}/p3c-content", group_id)
        )
        .execute(db.pool())
        .await
        .expect("seed the library file");
        file_id
    }

    async fn seed_metadata_index(db: &Database, group_id: i64, data_type: &str) -> Uuid {
        let index = db
            .create_metadata_index(&NewMetadataIndex {
                index_id: Uuid::new_v4(),
                group_id,
                source_key: "file_library",
                field_path: "section_score",
                data_type,
                value_kind: "scalar",
                sortable: false,
            })
            .await
            .expect("create the metadata index");
        db.finish_metadata_index(index.index_id, 0)
            .await
            .expect("mark the metadata index ready");
        index.index_id
    }

    async fn cleanup_content(db: &Database, file_id: Uuid, group_id: i64) {
        sqlx::query_file!(
            "tests/sql/task_execution/delete_file_document_chunks.sql",
            file_id.to_string()
        )
        .execute(db.pool())
        .await
        .expect("delete the document chunks");
        sqlx::query_file!(
            "tests/sql/task_execution/delete_file_document_metadata_values.sql",
            file_id.to_string()
        )
        .execute(db.pool())
        .await
        .expect("delete the document metadata values");
        sqlx::query_file!(
            "tests/sql/task_execution/delete_file_document_versions.sql",
            file_id.to_string()
        )
        .execute(db.pool())
        .await
        .expect("delete the document versions");
        sqlx::query_file!(
            "tests/sql/task_execution/delete_file_document_links.sql",
            file_id
        )
        .execute(db.pool())
        .await
        .expect("delete the links");
        sqlx::query_file!(
            "tests/sql/task_execution/delete_file_documents.sql",
            file_id.to_string()
        )
        .execute(db.pool())
        .await
        .expect("delete the documents");
        sqlx::query_file!("tests/sql/task_execution/delete_group_files.sql", group_id)
            .execute(db.pool())
            .await
            .expect("delete the library files");
        sqlx::query_file!(
            "tests/sql/task_execution/delete_group_metadata_indexes.sql",
            group_id
        )
        .execute(db.pool())
        .await
        .expect("delete the metadata indexes");
    }

    /// The live owner writes the real content rows under its grant, and a
    /// crashed segment writes none of them.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_live_owner_writes_the_ingest_content_and_a_crashed_segment_writes_none() {
        let db = connect().await;
        let _guard = TEST_LOCK.lock().await;
        let user_id = seed_user(&db).await;
        let group_id = seed_group(&db).await;
        let (task_id, item_id) = seed_task(&db, user_id, group_id).await;
        let item = claim(&db, item_id).await;
        let fence = TaskWriteFence::from_claimed_item(&item).expect("the captured grant");
        let store = LibraryStore::new(db.clone());
        let file_id = seed_library_file(&db, group_id).await;
        let _index = seed_metadata_index(&db, group_id, "integer").await;

        let payload = content_payload(file_id, group_id, "p3c-record-hash");
        assert!(
            write_document(&db, fence, &payload)
                .await
                .expect("the live grant writes the document"),
            "the live grant commits the document"
        );
        let document_id: i64 = sqlx::query_file_scalar!(
            "tests/sql/task_execution/read_file_document_id.sql",
            file_id.to_string()
        )
        .fetch_one(db.pool())
        .await
        .expect("read the document id");
        assert!(
            write_chunks(
                &db,
                fence,
                document_id,
                "p3c-record-hash",
                &content_chunks(document_id, 3),
            )
            .await
            .expect("the live grant writes the chunks"),
            "the live grant commits the chunks"
        );
        let mappings = vec![LibraryFileDocumentRecord {
            file_id,
            document_id,
            group_id,
            visibility: Visibility::Public,
            section_key: "document".to_string(),
            section_label: "Document".to_string(),
            section_external_id: None,
            section_source_uri: None,
            section_published_at: None,
            section_metadata_json: json!({}),
            sort_order: 0,
        }];
        assert!(
            store
                .replace_file_documents_and_status_fenced(
                    fence,
                    file_id,
                    &mappings,
                    LibraryIngestStatus::Succeeded,
                    None,
                    true,
                )
                .await
                .expect("the live grant writes the links"),
            "the live grant commits the links and the status"
        );

        let committed = content_rows(&db, file_id).await;
        assert_eq!(committed.documents, 1, "the document row is written");
        assert_eq!(committed.versions, 1, "the document version is written");
        assert_eq!(
            committed.metadata_values, 1,
            "the metadata value of the document is written"
        );
        assert_eq!(committed.chunks, 3, "the chunks are written");
        assert_eq!(committed.links, 1, "the link of the file is written");
        assert_eq!(committed.status, "succeeded", "the status is projected");

        // The crashed segment writes none of them.
        crash(&db, &item).await;
        let stale = content_payload(file_id, group_id, "p3c-record-hash-2");
        assert!(
            !write_document(&db, fence, &stale)
                .await
                .expect("a refused write is not an error"),
            "a crashed segment cannot write the document"
        );
        assert!(
            !write_chunks(
                &db,
                fence,
                document_id,
                "p3c-record-hash-2",
                &content_chunks(document_id, 1),
            )
            .await
            .expect("a refused write is not an error"),
            "a crashed segment cannot write the chunks"
        );
        assert!(
            !store
                .replace_file_documents_and_status_fenced(
                    fence,
                    file_id,
                    &mappings,
                    LibraryIngestStatus::Failed,
                    Some("crashed"),
                    false,
                )
                .await
                .expect("a refused write is not an error"),
            "a crashed segment cannot write the links"
        );
        assert_eq!(
            content_rows(&db, file_id).await,
            committed,
            "the committed content is preserved"
        );

        cleanup_content(&db, file_id, group_id).await;
        cleanup(&db, task_id, user_id, group_id).await;
    }

    /// A segment that lost its claim to a newer generation cannot overwrite the
    /// content of that generation.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn a_stale_generation_cannot_overwrite_the_content_of_its_successor() {
        let db = connect().await;
        let _guard = TEST_LOCK.lock().await;
        let user_id = seed_user(&db).await;
        let group_id = seed_group(&db).await;
        let (task_id, item_id) = seed_task(&db, user_id, group_id).await;
        let first = claim(&db, item_id).await;
        let first_fence = TaskWriteFence::from_claimed_item(&first).expect("the captured grant");
        let file_id = seed_library_file(&db, group_id).await;
        let _index = seed_metadata_index(&db, group_id, "integer").await;

        let first_payload = content_payload(file_id, group_id, "p3c-record-hash");
        assert!(
            write_document(&db, first_fence, &first_payload)
                .await
                .expect("the live grant writes the document"),
            "the first segment writes the document"
        );
        let committed = content_rows(&db, file_id).await;

        // The segment crashes and a newer generation takes the item over.
        crash(&db, &first).await;
        let second = claim(&db, item_id).await;
        let second_fence = TaskWriteFence::from_claimed_item(&second).expect("the captured grant");
        assert!(
            second_fence.generation() > first_fence.generation(),
            "the takeover runs under a newer generation"
        );

        // The older generation tries to write the same document again.
        let stale_payload = content_payload(file_id, group_id, "p3c-record-hash-2");
        assert!(
            !write_document(&db, first_fence, &stale_payload)
                .await
                .expect("a refused write is not an error"),
            "an older generation cannot write the document"
        );
        assert_eq!(
            content_rows(&db, file_id).await,
            committed,
            "the content of the live owner is preserved"
        );

        // The newer generation writes its own version.
        assert!(
            write_document(&db, second_fence, &stale_payload)
                .await
                .expect("the live grant writes the document"),
            "the newer generation writes the document"
        );
        let rows = content_rows(&db, file_id).await;
        assert_eq!(rows.documents, 1, "the document row is unchanged");
        assert_eq!(rows.versions, 2, "the newer generation adds its version");

        cleanup_content(&db, file_id, group_id).await;
        cleanup(&db, task_id, user_id, group_id).await;
    }

    /// An injected failure rolls the document write back: the document, its
    /// version and its metadata values commit together or not at all.
    #[tokio::test]
    #[ignore = "requires configured reusable PostgreSQL"]
    async fn an_injected_failure_rolls_the_document_write_back() {
        let db = connect().await;
        let _guard = TEST_LOCK.lock().await;
        let user_id = seed_user(&db).await;
        let group_id = seed_group(&db).await;
        let (task_id, item_id) = seed_task(&db, user_id, group_id).await;
        let item = claim(&db, item_id).await;
        let fence = TaskWriteFence::from_claimed_item(&item).expect("the captured grant");
        let file_id = seed_library_file(&db, group_id).await;
        // A metadata index whose declared type does not match the value: the
        // extraction fails inside the guarded write of the document.
        let _index = seed_metadata_index(&db, group_id, "keyword").await;

        let payload = content_payload(file_id, group_id, "p3c-record-hash");
        let failed = write_document(&db, fence, &payload).await;
        assert!(failed.is_err(), "the injected failure fails the write");

        let rows = content_rows(&db, file_id).await;
        assert_eq!(rows.documents, 0, "the document write is rolled back");
        assert_eq!(rows.versions, 0, "no version survives the rollback");
        assert_eq!(rows.metadata_values, 0, "no metadata value survives");
        assert_eq!(rows.chunks, 0, "no chunk is written");
        assert_eq!(rows.links, 0, "no link is written");
        assert_eq!(rows.status, "failed", "the status of the file is untouched");

        cleanup_content(&db, file_id, group_id).await;
        cleanup(&db, task_id, user_id, group_id).await;
    }
}
