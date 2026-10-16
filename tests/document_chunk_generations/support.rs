//! Fixtures for the issue #910 P3 chunk source-generation tests.
//!
//! Seeding and cleanup only; assertions stay in the case files. The tests run
//! only through `./scripts/test-backend.sh`, which supplies the validated test
//! target through `CONTEXT69_TEST_DATABASE_URL`.

use context69::db::Database;
use context69::domain::AccessScope;
use sqlx::Row as _;
use uuid::Uuid;

pub async fn connect_scratch() -> Database {
    let url = context69_test_support::accessor::database_url();
    Database::connect(&url)
        .await
        .expect("connect test database")
}

/// Seed a group with a unique public path.
pub async fn seed_group(db: &Database) -> i64 {
    let suffix = Uuid::new_v4().simple().to_string();
    sqlx::query(
        "INSERT INTO context69.groups (group_key, name, visibility, kind, full_path) \
         VALUES ($1, $2, 'public', 'personal', $3) RETURNING id",
    )
    .bind(format!("chunk-gen-{suffix}"))
    .bind(format!("Chunk Gen {suffix}"))
    .bind(format!("chunk-gen-{suffix}"))
    .fetch_one(db.pool())
    .await
    .expect("seed group")
    .get("id")
}

/// Seed a document in `group_id` with the given header `record_hash`.
pub async fn seed_document(db: &Database, group_id: i64, record_hash: &str) -> i64 {
    let suffix = Uuid::new_v4().simple().to_string();
    sqlx::query(
        "INSERT INTO context69.documents \
         (group_id, visibility, source_key, external_id, title, summary, source_uri, \
          updated_at_source, metadata_json, record_hash) \
         VALUES ($1, 'public', 'chunk-gen-src', $2, 'Chunk Gen Doc', NULL, \
                 'https://example.test/doc', now(), '{}'::jsonb, $3) RETURNING id",
    )
    .bind(group_id)
    .bind(format!("ext-{suffix}"))
    .bind(record_hash)
    .fetch_one(db.pool())
    .await
    .expect("seed document")
    .get("id")
}

/// Seed an original-locale chunk carrying its own stored `record_hash`.
pub async fn seed_chunk(
    db: &Database,
    document_id: i64,
    chunk_index: i32,
    chunk_text: &str,
    record_hash: &str,
) {
    sqlx::query(
        "INSERT INTO context69.document_chunks \
         (id, document_id, chunk_index, chunk_text, record_hash) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(Uuid::new_v4())
    .bind(document_id)
    .bind(chunk_index)
    .bind(chunk_text)
    .bind(record_hash)
    .execute(db.pool())
    .await
    .expect("seed chunk");
}

/// Seed a translation version whose `source_record_hash` is the generation the
/// translated text was produced from.
pub async fn seed_translation_version(
    db: &Database,
    document_id: i64,
    target_locale: &str,
    source_record_hash: &str,
    translated_title: &str,
) -> Uuid {
    sqlx::query(
        "INSERT INTO context69.document_translation_versions \
         (id, document_id, target_locale, source_locale, source_record_hash, provider_key, \
          provider_config_hash, model_name, translated_title, translated_summary, \
          translated_body_text) \
         VALUES ($1, $2, $3, NULL, $4, 'test-provider', 'test-config', NULL, $5, NULL, $6) \
         RETURNING id",
    )
    .bind(Uuid::new_v4())
    .bind(document_id)
    .bind(target_locale)
    .bind(source_record_hash)
    .bind(translated_title)
    .bind(translated_title)
    .fetch_one(db.pool())
    .await
    .expect("seed translation version")
    .get("id")
}

/// Seed one translated chunk of `translation_id`.
pub async fn seed_translation_chunk(
    db: &Database,
    translation_id: Uuid,
    document_id: i64,
    target_locale: &str,
    chunk_index: i32,
    chunk_text: &str,
) {
    sqlx::query(
        "INSERT INTO context69.document_translation_chunks \
         (id, translation_id, document_id, target_locale, chunk_index, chunk_text) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(Uuid::new_v4())
    .bind(translation_id)
    .bind(document_id)
    .bind(target_locale)
    .bind(chunk_index)
    .bind(chunk_text)
    .execute(db.pool())
    .await
    .expect("seed translation chunk");
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

pub async fn cleanup_group(db: &Database, group_id: i64) {
    sqlx::query("DELETE FROM context69.groups WHERE id = $1")
        .bind(group_id)
        .execute(db.pool())
        .await
        .ok();
}
