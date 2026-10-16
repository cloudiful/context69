//! Fixtures for the issue #910 P1 caller-produced document result tests.
//!
//! Seeding and cleanup only; assertions stay in the case files. The tests run
//! only through `./scripts/test-backend.sh`, which supplies the validated test
//! target through `CONTEXT69_TEST_DATABASE_URL`.

use context69::db::Database;
use context69::domain::AccessScope;
use serde_json::{Value, json};
use sqlx::Row as _;
use uuid::Uuid;

pub async fn connect_scratch() -> Database {
    let url = context69_test_support::accessor::database_url();
    Database::connect(&url)
        .await
        .expect("connect test database")
}

/// Seed a group with a unique path. `visibility` is `public` or `private`.
pub async fn seed_group(db: &Database, visibility: &str) -> i64 {
    let suffix = Uuid::new_v4().simple().to_string();
    let group_key = format!("results-{suffix}");
    let full_path = format!("results-{suffix}");
    sqlx::query(
        "INSERT INTO context69.groups (group_key, name, visibility, kind, full_path) \
         VALUES ($1, $2, $3, 'personal', $4) RETURNING id",
    )
    .bind(&group_key)
    .bind(format!("Results {suffix}"))
    .bind(visibility)
    .bind(&full_path)
    .fetch_one(db.pool())
    .await
    .expect("seed group")
    .get("id")
}

/// Seed a user; results reference it through the RESTRICT author FK.
pub async fn seed_user(db: &Database) -> i64 {
    sqlx::query(
        "INSERT INTO context69.users (login_name, display_name, password_hash) \
         VALUES ($1, 'Results Test', 'unused') RETURNING id",
    )
    .bind(format!("results-{}", Uuid::new_v4()))
    .fetch_one(db.pool())
    .await
    .expect("seed user")
    .get("id")
}

/// Seed a document in `group_id` with the given `record_hash` and metadata.
pub async fn seed_document(
    db: &Database,
    group_id: i64,
    record_hash: &str,
    metadata_json: &Value,
) -> i64 {
    let suffix = Uuid::new_v4().simple().to_string();
    sqlx::query(
        "INSERT INTO context69.documents \
         (group_id, visibility, source_key, external_id, title, summary, source_uri, \
          updated_at_source, metadata_json, record_hash) \
         VALUES ($1, 'public', 'results-src', $2, 'Result Doc', NULL, 'https://example.test/doc', \
                 now(), $3, $4) RETURNING id",
    )
    .bind(group_id)
    .bind(format!("ext-{suffix}"))
    .bind(metadata_json)
    .bind(record_hash)
    .fetch_one(db.pool())
    .await
    .expect("seed document")
    .get("id")
}

/// A caller-facing scope whose only visible group is `group_id`.
pub fn member_scope(group_id: i64, group_path: &str) -> AccessScope {
    AccessScope {
        user_id: None,
        include_public: true,
        private_group_ids: vec![group_id],
        group_path: Some(group_path.to_string()),
        scoped_group_id: Some(group_id),
    }
}

pub async fn group_path(db: &Database, group_id: i64) -> String {
    sqlx::query_scalar("SELECT full_path FROM context69.groups WHERE id = $1")
        .bind(group_id)
        .fetch_one(db.pool())
        .await
        .expect("group path")
}

pub async fn document_metadata(db: &Database, document_id: i64) -> Value {
    sqlx::query_scalar("SELECT metadata_json FROM context69.documents WHERE id = $1")
        .bind(document_id)
        .fetch_one(db.pool())
        .await
        .expect("document metadata")
}

pub async fn document_record_hash(db: &Database, document_id: i64) -> String {
    sqlx::query_scalar("SELECT record_hash FROM context69.documents WHERE id = $1")
        .bind(document_id)
        .fetch_one(db.pool())
        .await
        .expect("document record hash")
}

/// Move a document to a new source version, modelling a later ingestion.
pub async fn update_document_record_hash(db: &Database, document_id: i64, record_hash: &str) {
    sqlx::query(
        "UPDATE context69.documents SET record_hash = $2, updated_at_source = now() \
         WHERE id = $1",
    )
    .bind(document_id)
    .bind(record_hash)
    .execute(db.pool())
    .await
    .expect("update record hash");
}

pub async fn cleanup_group(db: &Database, group_id: i64) {
    sqlx::query("DELETE FROM context69.groups WHERE id = $1")
        .bind(group_id)
        .execute(db.pool())
        .await
        .ok();
}

pub async fn cleanup_user(db: &Database, user_id: i64) {
    sqlx::query("DELETE FROM context69.users WHERE id = $1")
        .bind(user_id)
        .execute(db.pool())
        .await
        .ok();
}

/// A minimal valid envelope bound to `record_hash`.
pub fn result_request(producer: &str, key: &str, record_hash: &str) -> Value {
    json!({
        "producer": producer,
        "result_kind": "sentiment.v1",
        "schema_ref": "stock.news/sentiment@1",
        "source_record_hash": record_hash,
        "idempotency_key": key,
        "result": {"direction": "positive"},
        "provenance": {"run_id": "run-1"}
    })
}
