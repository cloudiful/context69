//! Shared fixtures for the issue 139 version-invariant regression.
//!
//! Seeding and assertion helpers only; behavior and isolation match the
//! original single-file test. Extraction read-path assertions stay in the
//! parent test file.

use context69::db::Database;
use sqlx::Row;
use uuid::Uuid;

pub fn test_database_url() -> Option<String> {
    std::env::var("CONTEXT69_TEST_DATABASE_URL").ok()
}

pub async fn seed_group(db: &Database) -> i64 {
    let key = format!("version-invariant-{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO context69.groups \
         (group_key, name, visibility, kind, full_path) \
         VALUES ($1, $2, 'public', 'shared', $3) RETURNING id",
    )
    .bind(&key)
    .bind("Version Invariant Test Group")
    .bind(format!("test/version-invariant-{key}"))
    .fetch_one(db.pool())
    .await
    .expect("seed test group")
    .get("id")
}

pub async fn seed_document_with_chunks(
    db: &Database,
    group_id: i64,
    hash: &str,
    title: &str,
    chunk_texts: &[&str],
) -> i64 {
    let document_id: i64 = sqlx::query(
        "INSERT INTO context69.documents \
         (group_id, source_key, external_id, title, summary, source_uri, \
          updated_at_source, record_hash, metadata_json, visibility) \
         VALUES ($1, 'version-invariant-test', $2, $3, 'invariant summary', \
          'https://example.test/version-invariant', now(), $4, '{}'::jsonb, 'public') \
         RETURNING id",
    )
    .bind(group_id)
    .bind(format!("version-invariant-{}", Uuid::new_v4()))
    .bind(title)
    .bind(hash)
    .fetch_one(db.pool())
    .await
    .expect("seed test document")
    .get("id");

    // Insert out of order on purpose: the snapshot must reconstruct the body
    // from chunks ordered by `chunk_index`, not insertion order.
    let mut ordered: Vec<(i32, &str)> = chunk_texts
        .iter()
        .enumerate()
        .map(|(index, text)| (index as i32, *text))
        .collect();
    ordered.reverse();
    for (chunk_index, chunk_text) in ordered {
        sqlx::query(
            "INSERT INTO context69.document_chunks \
             (id, document_id, chunk_index, chunk_text, record_hash) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(Uuid::new_v4())
        .bind(document_id)
        .bind(chunk_index)
        .bind(chunk_text)
        .bind(hash)
        .execute(db.pool())
        .await
        .expect("seed test chunk");
    }
    document_id
}

pub async fn cleanup_group(db: &Database, group_id: i64) {
    sqlx::query(
        "DELETE FROM context69.document_chunks WHERE document_id IN \
         (SELECT id FROM context69.documents WHERE group_id = $1)",
    )
    .bind(group_id)
    .execute(db.pool())
    .await
    .expect("clean up chunks");
    sqlx::query("DELETE FROM context69.documents WHERE group_id = $1")
        .bind(group_id)
        .execute(db.pool())
        .await
        .expect("clean up documents");
    sqlx::query("DELETE FROM context69.groups WHERE id = $1")
        .bind(group_id)
        .execute(db.pool())
        .await
        .expect("clean up group");
}

pub fn business_payload(
    document_id: i64,
    group_id: i64,
    record_hash: &str,
    metadata_json: serde_json::Value,
) -> context69::domain::ChunkPayload {
    context69::domain::ChunkPayload {
        chunk_id: Uuid::nil(),
        document_id,
        group_id,
        group_key: "version-invariant-test".to_string(),
        group_path: "test/version-invariant".to_string(),
        visibility: context69_contracts::Visibility::Public,
        source_key: "version-invariant-test".to_string(),
        external_id: format!("version-invariant-{}", Uuid::new_v4()),
        title: "invariant title".to_string(),
        summary: Some("invariant summary".to_string()),
        source_uri: "https://example.test/version-invariant/updated".to_string(),
        published_at: None,
        updated_at_source: chrono::Utc::now(),
        record_hash: record_hash.to_string(),
        chunk_index: 0,
        // Deliberately partial: the snapshot must ignore this and rebuild the
        // complete body from ordered chunks.
        chunk_text: "only first chunk".to_string(),
        metadata_json,
        content_locale: "original".to_string(),
        source_locale: None,
        translation_provider: None,
    }
}

pub async fn document_hash(db: &Database, document_id: i64) -> String {
    sqlx::query_scalar("SELECT record_hash FROM context69.documents WHERE id = $1")
        .bind(document_id)
        .fetch_one(db.pool())
        .await
        .expect("load document hash")
}

pub async fn version_body(db: &Database, document_id: i64, hash: &str) -> Option<String> {
    sqlx::query_scalar(
        "SELECT body_text FROM context69.document_versions \
         WHERE document_id = $1 AND record_hash = $2",
    )
    .bind(document_id)
    .bind(hash)
    .fetch_optional(db.pool())
    .await
    .expect("load version body")
}

pub async fn version_count(db: &Database, document_id: i64) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM context69.document_versions WHERE document_id = $1")
        .bind(document_id)
        .fetch_one(db.pool())
        .await
        .expect("count versions")
}
