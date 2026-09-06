//! Shared scratch-DB fixtures for the issue 139 phase 4 backfill tests.
//!
//! Seeding and assertion helpers only; all safety-behavior assertions
//! stay in the parent test target. Helpers never touch the normal
//! `.env` dev database; they use only the caller-supplied pool.

use chrono::{DateTime, Utc};
use context69::db::Database;
use context69::domain::SourceRecord;
use context69::normalize::normalize_record;
use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

pub fn test_database_url() -> Option<String> {
    std::env::var("CONTEXT69_TEST_DATABASE_URL").ok()
}

pub async fn seed_group(db: &Database) -> i64 {
    let key = format!("backfill-{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO context69.groups (group_key, name, visibility, kind, full_path) \
         VALUES ($1, $2, 'public', 'shared', $3) RETURNING id",
    )
    .bind(&key)
    .bind("Backfill Test Group")
    .bind(format!("test/backfill-{key}"))
    .fetch_one(db.pool())
    .await
    .expect("seed test group")
    .get("id")
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

pub struct SeedDoc<'a> {
    pub source_key: &'a str,
    pub title: &'a str,
    pub summary: Option<&'a str>,
    pub source_uri: &'a str,
    pub published_at: Option<DateTime<Utc>>,
    pub metadata: Value,
    pub chunk_texts: &'a [&'a str],
    pub record_hash_override: Option<&'a str>,
    pub reverse_insert: bool,
}

pub async fn seed_document(
    db: &Database,
    group_id: i64,
    options: SeedDoc<'_>,
) -> (i64, String, String) {
    let raw_body = options.chunk_texts.join("\n");
    let hash = match options.record_hash_override {
        Some(value) => value.to_string(),
        None => {
            normalize_record(SourceRecord {
                external_id: format!("backfill-{}", Uuid::new_v4()),
                title: options.title.to_string(),
                body_text: raw_body.clone(),
                source_uri: options.source_uri.to_string(),
                summary: options.summary.map(ToOwned::to_owned),
                published_at: options.published_at,
                updated_at: Utc::now(),
                metadata_json: options.metadata.clone(),
            })
            .record_hash
        }
    };
    let document_id: i64 = sqlx::query(
        "INSERT INTO context69.documents \
         (group_id, source_key, external_id, title, summary, source_uri, published_at, \
          updated_at_source, record_hash, metadata_json, visibility) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, now(), $8, $9, 'public') RETURNING id",
    )
    .bind(group_id)
    .bind(options.source_key)
    .bind(format!("backfill-{}", Uuid::new_v4()))
    .bind(options.title)
    .bind(options.summary)
    .bind(options.source_uri)
    .bind(options.published_at)
    .bind(&hash)
    .bind(&options.metadata)
    .fetch_one(db.pool())
    .await
    .expect("seed test document")
    .get("id");

    let mut ordered: Vec<(i32, &str)> = options
        .chunk_texts
        .iter()
        .enumerate()
        .map(|(index, text)| (index as i32, *text))
        .collect();
    if options.reverse_insert {
        ordered.reverse();
    }
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
        .bind(&hash)
        .execute(db.pool())
        .await
        .expect("seed test chunk");
    }
    (document_id, hash, raw_body)
}

/// Concise helper for the common eligible `file_library` case with
/// default metadata and no hash override.
pub async fn seed_file_library(
    db: &Database,
    group_id: i64,
    title: &str,
    source_uri: &str,
    chunk_texts: &[&str],
) -> (i64, String, String) {
    seed_document(
        db,
        group_id,
        SeedDoc {
            source_key: "file_library",
            title,
            summary: None,
            source_uri,
            published_at: None,
            metadata: serde_json::json!({}),
            chunk_texts,
            record_hash_override: None,
            reverse_insert: false,
        },
    )
    .await
}

/// Concise helper for an intentionally unrepairable `file_library`
/// document with an explicit stored hash.
pub async fn seed_unrepairable(
    db: &Database,
    group_id: i64,
    title: &str,
    source_uri: &str,
    chunk_texts: &[&str],
    stored_hash: &str,
) -> i64 {
    seed_document(
        db,
        group_id,
        SeedDoc {
            source_key: "file_library",
            title,
            summary: None,
            source_uri,
            published_at: None,
            metadata: serde_json::json!({}),
            chunk_texts,
            record_hash_override: Some(stored_hash),
            reverse_insert: false,
        },
    )
    .await
    .0
}

pub async fn version_count(db: &Database, document_id: i64) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM context69.document_versions WHERE document_id = $1")
        .bind(document_id)
        .fetch_one(db.pool())
        .await
        .expect("count versions")
}
