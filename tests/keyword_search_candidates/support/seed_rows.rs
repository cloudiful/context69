use chrono::{DateTime, Utc};
use context69::db::Database;
use serde_json::Value;
use uuid::Uuid;

pub struct Document {
    pub id: i64,
    pub record_hash: String,
}

pub struct DocumentOptions<'a> {
    pub summary: Option<&'a str>,
    pub metadata: Value,
    pub published_at: Option<DateTime<Utc>>,
    pub visibility: &'a str,
}

pub async fn group(
    db: &Database,
    group_key: &str,
    name: &str,
    visibility: &str,
    full_path: &str,
) -> i64 {
    sqlx::query_scalar::<_, i64>(include_str!("../fixtures/seed.sql"))
        .bind(group_key)
        .bind(name)
        .bind(visibility)
        .bind(full_path)
        .fetch_one(db.pool())
        .await
        .expect("insert issue-owned synthetic group")
}

pub async fn document(
    db: &Database,
    group_id: i64,
    source_key: &str,
    external_id: &str,
    title: &str,
    options: DocumentOptions<'_>,
) -> Document {
    let record_hash = format!("issue62-{external_id}");
    let id = sqlx::query_scalar::<_, i64>(
        "INSERT INTO context69.documents \
         (group_id, source_key, external_id, title, summary, source_uri, published_at, \
          updated_at_source, metadata_json, record_hash, visibility) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, now(), $8, $9, $10) RETURNING id",
    )
    .bind(group_id)
    .bind(source_key)
    .bind(external_id)
    .bind(title)
    .bind(options.summary)
    .bind(format!("https://example.test/issue62/{external_id}"))
    .bind(options.published_at)
    .bind(options.metadata)
    .bind(&record_hash)
    .bind(options.visibility)
    .fetch_one(db.pool())
    .await
    .expect("insert issue-owned synthetic document");
    Document { id, record_hash }
}

pub async fn chunk(
    db: &Database,
    document: &Document,
    chunk_index: i32,
    chunk_text: &str,
    id: Option<Uuid>,
) {
    sqlx::query(
        "INSERT INTO context69.document_chunks (id, document_id, chunk_index, chunk_text, record_hash) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(id.unwrap_or_else(Uuid::new_v4))
    .bind(document.id)
    .bind(chunk_index)
    .bind(chunk_text)
    .bind(&document.record_hash)
    .execute(db.pool())
    .await
    .expect("insert issue-owned synthetic document chunk");
}

pub async fn translation_version(
    db: &Database,
    id: Uuid,
    document: &Document,
    target_locale: &str,
    source_record_hash: &str,
    translated_title: &str,
    translated_summary: Option<&str>,
) -> Uuid {
    sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO context69.document_translation_versions \
         (id, document_id, target_locale, source_locale, source_record_hash, provider_key, \
          provider_config_hash, translated_title, translated_summary, translated_body_text) \
         VALUES ($1, $2, $3, 'en', $4, 'llm', 'synthetic', $5, $6, $7) RETURNING id",
    )
    .bind(id)
    .bind(document.id)
    .bind(target_locale)
    .bind(source_record_hash)
    .bind(translated_title)
    .bind(translated_summary)
    .bind("synthetic translation body")
    .fetch_one(db.pool())
    .await
    .expect("insert issue-owned synthetic translation version")
}

pub async fn translated_chunk(
    db: &Database,
    id: Uuid,
    translation_id: Uuid,
    document_id: i64,
    locale: &str,
    chunk_index: i32,
    chunk_text: &str,
) {
    sqlx::query(
        "INSERT INTO context69.document_translation_chunks \
         (id, translation_id, document_id, target_locale, chunk_index, chunk_text) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(id)
    .bind(translation_id)
    .bind(document_id)
    .bind(locale)
    .bind(chunk_index)
    .bind(chunk_text)
    .execute(db.pool())
    .await
    .expect("insert issue-owned synthetic translated chunk");
}

pub async fn translation_job(
    db: &Database,
    document_id: i64,
    locale: &str,
    source_record_hash: &str,
    status: &str,
    updated_at: DateTime<Utc>,
) {
    sqlx::query(
        "INSERT INTO context69.document_translation_jobs \
         (id, document_id, target_locale, requested_source_locale, source_record_hash, status, \
          created_at, updated_at) \
         VALUES ($1, $2, $3, 'en', $4, $5, $6, $6)",
    )
    .bind(Uuid::new_v4())
    .bind(document_id)
    .bind(locale)
    .bind(source_record_hash)
    .bind(status)
    .bind(updated_at)
    .execute(db.pool())
    .await
    .expect("insert issue-owned synthetic translation job");
}
