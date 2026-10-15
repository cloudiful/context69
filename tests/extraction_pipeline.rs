//! Required-extraction gate regression (issue #89 P3).
//!
//! Validates actual DB behavior, not compilation only: the dedicated
//! `extraction_required` flag persists independently of caller-owned
//! `extraction_parameters`, the migration default (`TRUE`) never creates a
//! directive on its own, and the owned CAS mark only fails a `succeeded` file
//! while the worker still owns the bound task item. All tests are gated by
//! `CONTEXT69_TEST_DATABASE_URL` and registered in the static inventory.

use context69::db::Database;
use context69::library_store::LibraryStore;
use sqlx::Row;
use uuid::Uuid;

static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn db() -> Database {
    let url = context69_test_support::accessor::database_url();
    Database::connect(&url)
        .await
        .expect("connect test database")
}

async fn seed_user(db: &Database) -> i64 {
    sqlx::query(
        "INSERT INTO context69.users (login_name, display_name, password_hash) VALUES ($1,$2,$3) RETURNING id",
    )
    .bind(format!("extract-gate-{}", Uuid::new_v4()))
    .bind("Extraction Gate")
    .bind("unused")
    .fetch_one(db.pool())
    .await
    .expect("seed user")
    .get("id")
}

async fn seed_group(db: &Database) -> i64 {
    let k = format!("g-{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO context69.groups (group_key, name, visibility, kind, full_path) VALUES ($1,$2,'public','personal',$3) RETURNING id",
    )
    .bind(&k)
    .bind(&k)
    .bind(format!("/{k}"))
    .fetch_one(db.pool())
    .await
    .expect("seed group")
    .get("id")
}

async fn seed_file(db: &Database, group_id: i64, ingest_status: &str) -> Uuid {
    let file_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO context69.library_files (id, group_id, visibility, folder_id, external_id, filename, media_type, size_bytes, sha256, storage_rel_path, storage_object_id, ingest_status) VALUES ($1,$2,'public',NULL,$3,$4,'text/plain',11,$5,$6,NULL,$7)",
    )
    .bind(file_id)
    .bind(group_id)
    .bind(format!("ext-{}", Uuid::new_v4()))
    .bind(format!("gate-{}.txt", Uuid::new_v4()))
    .bind(format!("sha-{}", Uuid::new_v4()))
    .bind(format!("test/gate-{}", Uuid::new_v4()))
    .bind(ingest_status)
    .execute(db.pool())
    .await
    .expect("seed file");
    file_id
}

async fn seed_owned_item(
    db: &Database,
    user_id: i64,
    group_id: i64,
    file_id: Uuid,
    lease: Uuid,
) -> (Uuid, Uuid) {
    let task_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO context69.tasks (id, user_id, group_id, kind, status, total_count) VALUES ($1,$2,$3,'file_batch','running',1)",
    )
    .bind(task_id)
    .bind(user_id)
    .bind(group_id)
    .execute(db.pool())
    .await
    .expect("seed task");
    let item_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO context69.task_items (id, task_id, ordinal, payload, status, file_id, lease_token, lease_until) VALUES ($1,$2,0,'{}','running',$3,$4,now()+interval '5 minutes')",
    )
    .bind(item_id)
    .bind(task_id)
    .bind(file_id)
    .bind(lease)
    .execute(db.pool())
    .await
    .expect("seed item");
    (task_id, item_id)
}

async fn cleanup(db: &Database, task_id: Uuid, item_id: Uuid, file_id: Uuid, group_id: i64) {
    sqlx::query("DELETE FROM context69.task_items WHERE id=$1")
        .bind(item_id)
        .execute(db.pool())
        .await
        .ok();
    sqlx::query("DELETE FROM context69.tasks WHERE id=$1")
        .bind(task_id)
        .execute(db.pool())
        .await
        .ok();
    sqlx::query("DELETE FROM context69.library_files WHERE id=$1")
        .bind(file_id)
        .execute(db.pool())
        .await
        .ok();
    sqlx::query("DELETE FROM context69.groups WHERE id=$1")
        .bind(group_id)
        .execute(db.pool())
        .await
        .ok();
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn directive_required_persists_in_separate_flag() {
    let _g = LOCK.lock().await;
    let db = db().await;
    let store = LibraryStore::new(db.clone());
    let group_id = seed_group(&db).await;
    let file_id = seed_file(&db, group_id, "succeeded").await;

    // Arbitrary parameters round-trip unchanged while the flag varies.
    let params = serde_json::json!({ "depth": "fast", "nested": { "k": [1, 2] } });
    store
        .set_file_extraction_directive(
            file_id,
            Some(&context69::contracts::ExtractionDirective {
                template_key: "t-1".to_string(),
                parameters: params.clone(),
                required: false,
            }),
        )
        .await
        .expect("set optional");
    let got = store
        .file_extraction_directive(file_id)
        .await
        .expect("get optional")
        .expect("directive");
    assert_eq!(got.template_key, "t-1");
    assert_eq!(got.parameters, params);
    assert!(!got.required);

    store
        .set_file_extraction_directive(
            file_id,
            Some(&context69::contracts::ExtractionDirective {
                template_key: "t-1".to_string(),
                parameters: params.clone(),
                required: true,
            }),
        )
        .await
        .expect("set required");
    let got = store
        .file_extraction_directive(file_id)
        .await
        .expect("get required")
        .expect("directive");
    assert!(got.required);
    assert_eq!(got.parameters, params);

    sqlx::query("DELETE FROM context69.library_files WHERE id=$1")
        .bind(file_id)
        .execute(db.pool())
        .await
        .ok();
    sqlx::query("DELETE FROM context69.groups WHERE id=$1")
        .bind(group_id)
        .execute(db.pool())
        .await
        .ok();
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn migration_default_true_creates_no_directive() {
    let _g = LOCK.lock().await;
    let db = db().await;
    let store = LibraryStore::new(db.clone());
    let group_id = seed_group(&db).await;
    let file_id = seed_file(&db, group_id, "succeeded").await;

    // The `TRUE` default is policy only: a NULL template key still means no
    // extraction work.
    assert!(
        store
            .file_extraction_directive(file_id)
            .await
            .expect("get")
            .is_none()
    );
    let flag: bool =
        sqlx::query_scalar("SELECT extraction_required FROM context69.library_files WHERE id=$1")
            .bind(file_id)
            .fetch_one(db.pool())
            .await
            .expect("flag");
    assert!(flag);

    sqlx::query("DELETE FROM context69.library_files WHERE id=$1")
        .bind(file_id)
        .execute(db.pool())
        .await
        .ok();
    sqlx::query("DELETE FROM context69.groups WHERE id=$1")
        .bind(group_id)
        .execute(db.pool())
        .await
        .ok();
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn owned_gate_marks_succeeded_file_failed() {
    let _g = LOCK.lock().await;
    let db = db().await;
    let store = LibraryStore::new(db.clone());
    let user_id = seed_user(&db).await;
    let group_id = seed_group(&db).await;
    let file_id = seed_file(&db, group_id, "succeeded").await;
    let lease = Uuid::new_v4();
    let (task_id, item_id) = seed_owned_item(&db, user_id, group_id, file_id, lease).await;

    let marked = store
        .mark_extraction_gate_failed(
            file_id,
            item_id,
            lease,
            "succeeded",
            "required extraction failed for template 't-1' (job 00000000-0000-0000-0000-000000000000)",
        )
        .await
        .expect("cas")
        .expect("owned mark lands");
    assert_eq!(marked.id, file_id);
    assert_eq!(
        marked.ingest_status,
        context69::contracts::LibraryIngestStatus::Failed
    );
    assert!(
        marked
            .error_message
            .as_deref()
            .unwrap_or_default()
            .contains("required extraction failed")
    );
    // No raw provider payload may leak through the fixed safe message.
    assert!(
        !marked
            .error_message
            .as_deref()
            .unwrap_or_default()
            .contains("api_key")
    );

    // The file already left `succeeded`: a second mark with the same
    // expectation is fenced (no write).
    assert!(
        store
            .mark_extraction_gate_failed(file_id, item_id, lease, "succeeded", "retry")
            .await
            .expect("second cas")
            .is_none()
    );

    cleanup(&db, task_id, item_id, file_id, group_id).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn stale_lease_and_cancelled_item_cannot_mark() {
    let _g = LOCK.lock().await;
    let db = db().await;
    let store = LibraryStore::new(db.clone());
    let user_id = seed_user(&db).await;
    let group_id = seed_group(&db).await;
    let file_id = seed_file(&db, group_id, "succeeded").await;
    let lease = Uuid::new_v4();
    let (task_id, item_id) = seed_owned_item(&db, user_id, group_id, file_id, lease).await;

    // Same-ID stale lease cannot mark a new owner's file.
    assert!(
        store
            .mark_extraction_gate_failed(file_id, item_id, Uuid::new_v4(), "succeeded", "stale")
            .await
            .expect("stale cas")
            .is_none()
    );

    // Cancellation/claim race: a non-running item is fenced even with the
    // correct lease.
    sqlx::query("UPDATE context69.task_items SET status='cancelled', updated_at=now() WHERE id=$1")
        .bind(item_id)
        .execute(db.pool())
        .await
        .expect("cancel item");
    assert!(
        store
            .mark_extraction_gate_failed(file_id, item_id, lease, "succeeded", "cancelled")
            .await
            .expect("cancelled cas")
            .is_none()
    );

    // The file is still `succeeded`: the fenced attempts wrote nothing.
    let row: String =
        sqlx::query_scalar("SELECT ingest_status FROM context69.library_files WHERE id=$1")
            .bind(file_id)
            .fetch_one(db.pool())
            .await
            .expect("status");
    assert_eq!(row, "succeeded");

    cleanup(&db, task_id, item_id, file_id, group_id).await;
}
