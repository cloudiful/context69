//! Issue 667 Phase 1: the staged object reference is only committed under the
//! running status and current lease token, so a stale worker cannot bind it.

use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

use crate::db::Database;

use super::super::fixtures::{cleanup, seed_group, seed_user};
use super::super::support::{PIPELINE_CASE_LOCK, build_service};
use super::{claim_item, create_url_task, sha256_hex, staged_url_payload, uploaded_file};

/// Issue 667 Phase 1: the staged object reference is only committed under the
/// running status and the current lease token, so a stale worker cannot bind
/// or rebind it.
#[tokio::test]
async fn staged_input_reference_requires_the_running_lease_token() {
    let Some(database_url) = std::env::var("CONTEXT69_TEST_DATABASE_URL").ok() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping issue 667 lease test");
        return;
    };
    let _guard = PIPELINE_CASE_LOCK.lock().await;
    let db = Database::connect(&database_url)
        .await
        .expect("connect test database");
    let user_id = seed_user(&db).await;
    let group = seed_group(&db).await;
    let (service, storage_root) = build_service(&db).await;

    let content = b"issue 667 guarded reference body";
    let sha256 = sha256_hex(content);
    let object_id = service
        .library()
        .stage_file_for_task_input(group.id, uploaded_file(&sha256, content, "issue-667-lease"))
        .await
        .expect("stage the streamed input object");
    let original = staged_url_payload(&sha256, "issue-667-lease");
    let (task_id, _reused, item_ids) = create_url_task(
        &db,
        user_id,
        group.id,
        std::slice::from_ref(&original),
        &[None],
    )
    .await;
    let item = claim_item(&db, item_ids[0]).await;
    let committed = staged_url_payload(&sha256, "issue-667-lease");

    let stale = db
        .set_task_item_input_storage_object(item.id, Uuid::new_v4(), object_id, &committed)
        .await
        .expect("stale guarded update");
    assert!(!stale, "a stale lease token must not commit the reference");

    let row = sqlx::query(
        "SELECT input_storage_object_id, payload FROM context69.task_items WHERE id = $1",
    )
    .bind(item.id)
    .fetch_one(db.pool())
    .await
    .expect("load the untouched item");
    assert_eq!(row.get::<Option<Uuid>, _>("input_storage_object_id"), None);
    assert_eq!(row.get::<Value, _>("payload"), original);

    let updated = db
        .set_task_item_input_storage_object(item.id, item.lease_token, object_id, &committed)
        .await
        .expect("current guarded update");
    assert!(updated, "the running lease must commit the reference");

    let row = sqlx::query(
        "SELECT input_storage_object_id, payload FROM context69.task_items WHERE id = $1",
    )
    .bind(item.id)
    .fetch_one(db.pool())
    .await
    .expect("load the committed item");
    assert_eq!(
        row.get::<Option<Uuid>, _>("input_storage_object_id"),
        Some(object_id)
    );
    assert_eq!(row.get::<Value, _>("payload"), committed);

    // The committed reference also owns the staging object: while an item
    // points at it, the guarded release refuses to delete it.
    service
        .library()
        .release_task_input_staging(object_id, None)
        .await
        .expect("the guarded release is a no-op while the item references the object");
    let retained: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM context69.library_storage_objects WHERE id = $1")
            .bind(object_id)
            .fetch_one(db.pool())
            .await
            .expect("load the retained object");
    assert_eq!(retained, Some(object_id));

    // A terminal item that dropped the reference releases the staging object,
    // which is what the task-item FK (ON DELETE RESTRICT) requires before the
    // group fixture can clean up.
    sqlx::query("UPDATE context69.task_items SET input_storage_object_id = NULL WHERE id = $1")
        .bind(item.id)
        .execute(db.pool())
        .await
        .expect("clear the committed reference");
    service
        .library()
        .release_task_input_staging(object_id, None)
        .await
        .expect("release the staged object");
    let released: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM context69.library_storage_objects WHERE id = $1")
            .bind(object_id)
            .fetch_optional(db.pool())
            .await
            .expect("load the released object");
    assert_eq!(released, None, "the staged object is reclaimed");

    let _ = task_id;
    cleanup(&db, group.id, user_id).await;
    let _ = std::fs::remove_dir_all(storage_root);
}
