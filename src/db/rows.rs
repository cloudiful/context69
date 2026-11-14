use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

use crate::{contracts::SearchHit, normalize::is_meaningful_text};

#[derive(Debug, Clone, FromRow)]
pub(super) struct DocumentRow {
    pub(super) id: i64,
    pub(super) group_key: String,
    pub(super) group_path: String,
    pub(super) visibility: String,
    pub(super) source_key: String,
    pub(super) external_id: String,
    pub(super) title: String,
    pub(super) summary: Option<String>,
    pub(super) source_uri: String,
    pub(super) published_at: Option<DateTime<Utc>>,
    pub(super) updated_at_source: DateTime<Utc>,
    pub(super) record_hash: String,
    pub(super) metadata_json: Value,
}

#[derive(Debug, Clone, FromRow)]
pub(super) struct ChunkRow {
    pub(super) chunk_index: i32,
    pub(super) chunk_text: String,
}

#[derive(Debug, Clone, FromRow)]
pub(super) struct ReadDocumentChunkRow {
    pub(super) id: Uuid,
    pub(super) chunk_index: i32,
    pub(super) chunk_text: String,
    pub(super) record_hash: String,
}

#[derive(Debug, Clone, FromRow)]
pub(super) struct ReadDocumentChunkBatchRow {
    pub(super) document_id: i64,
    pub(super) id: Uuid,
    pub(super) chunk_index: i32,
    pub(super) chunk_text: String,
    pub(super) record_hash: String,
}

#[derive(Debug, Clone, FromRow)]
pub(super) struct TranslationVersionBatchRow {
    pub(super) document_id: i64,
    pub(super) id: Uuid,
    pub(super) target_locale: String,
    pub(super) translated_title: String,
    pub(super) translated_summary: Option<String>,
    pub(super) source_record_hash: String,
}

#[derive(Debug, Clone, FromRow)]
pub(super) struct TranslationChunkBatchRow {
    pub(super) translation_id: Uuid,
    pub(super) id: Uuid,
    pub(super) chunk_index: i32,
    pub(super) chunk_text: String,
}

#[derive(Debug, Clone, FromRow)]
pub(super) struct TranslationStatusBatchRow {
    pub(super) document_id: i64,
    pub(super) status: String,
    pub(super) source_locale: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub(super) struct SearchHitRow {
    pub(super) chunk_id: Uuid,
    pub(super) document_id: i64,
    pub(super) group_key: String,
    pub(super) group_path: String,
    pub(super) visibility: String,
    pub(super) source_key: String,
    pub(super) external_id: String,
    pub(super) title: String,
    pub(super) summary: Option<String>,
    pub(super) source_uri: String,
    pub(super) published_at: Option<DateTime<Utc>>,
    pub(super) chunk_index: i32,
    pub(super) chunk_text: String,
    pub(super) metadata_json: Value,
    pub(super) content_locale: String,
    pub(super) translation_status: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub(super) struct ReindexChunkRow {
    pub(super) chunk_id: Uuid,
    pub(super) document_id: i64,
    pub(super) group_id: i64,
    pub(super) group_key: String,
    pub(super) group_path: String,
    pub(super) visibility: String,
    pub(super) source_key: String,
    pub(super) external_id: String,
    pub(super) title: String,
    pub(super) summary: Option<String>,
    pub(super) source_uri: String,
    pub(super) published_at: Option<DateTime<Utc>>,
    pub(super) updated_at_source: DateTime<Utc>,
    pub(super) record_hash: String,
    pub(super) chunk_index: i32,
    pub(super) chunk_text: String,
    pub(super) metadata_json: Value,
}

#[derive(Debug, Clone, FromRow)]
pub(super) struct KeywordSearchHitRow {
    pub(super) chunk_id: Uuid,
    pub(super) document_id: i64,
    pub(super) group_key: String,
    pub(super) group_path: String,
    pub(super) visibility: String,
    pub(super) source_key: String,
    pub(super) external_id: String,
    pub(super) title: String,
    pub(super) summary: Option<String>,
    pub(super) source_uri: String,
    pub(super) published_at: Option<DateTime<Utc>>,
    pub(super) chunk_index: i32,
    pub(super) chunk_text: String,
    pub(super) metadata_json: Value,
    pub(super) keyword_score: f32,
    pub(super) match_reason: String,
    pub(super) content_locale: String,
    pub(super) translation_status: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub(super) struct RuntimeChunkingSettingsRow {
    pub(super) max_chars: i64,
    pub(super) overlap_chars: i64,
    pub(super) unit: String,
    pub(super) tokenizer: String,
    pub(super) tokenizer_sha256: Option<String>,
    pub(super) max_tokens: i64,
    pub(super) overlap_tokens: i64,
}

/// One stored source connection.
///
/// `name` is the user-controlled API/display identifier, `connection_key` the
/// stable identity this application minted, and `database_url_secret_key` the
/// `context69.internal_secrets` row that holds the sealed database URL. The URL
/// itself is only ever in that row, never in this table.
#[derive(Debug, Clone, FromRow)]
pub(super) struct SourceConnectionRow {
    pub(super) name: String,
    pub(super) connection_key: Uuid,
    pub(super) database_url_secret_key: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub(super) struct ExistingDocumentRow {
    pub(super) id: i64,
    pub(super) record_hash: String,
}

pub(super) fn is_library_file(metadata_json: &Value) -> bool {
    metadata_json
        .get("is_library_file")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

pub(super) fn search_hit_from_keyword_row(
    row: KeywordSearchHitRow,
    requested_locale: Option<&str>,
) -> SearchHit {
    SearchHit {
        chunk_id: row.chunk_id,
        document_id: row.document_id,
        group_key: row.group_key,
        group_path: row.group_path,
        visibility: row
            .visibility
            .parse()
            .unwrap_or(crate::contracts::Visibility::Private),
        source_key: row.source_key,
        external_id: row.external_id,
        title: row.title,
        summary: row.summary.filter(|value| is_meaningful_text(value)),
        source_uri: row.source_uri,
        published_at: row.published_at,
        chunk_index: row.chunk_index,
        chunk_text: row.chunk_text,
        score: row.keyword_score,
        vector_score: None,
        keyword_score: Some(row.keyword_score),
        rerank_score: None,
        match_reason: Some(row.match_reason),
        library_file_id: library_file_id(&row.metadata_json),
        library_section_label: library_section_label(&row.metadata_json),
        library_path: library_path(&row.metadata_json),
        is_library_file: is_library_file(&row.metadata_json),
        metadata_json: row.metadata_json,
        requested_locale: requested_locale.map(ToOwned::to_owned),
        content_locale: Some(row.content_locale.clone()),
        translation_status: translation_status(row.translation_status.as_deref()),
        is_fallback: requested_locale.is_some() && row.content_locale == "original",
    }
}

pub(super) fn translation_status(
    value: Option<&str>,
) -> Option<crate::contracts::TranslationStatus> {
    match value {
        Some("queued") => Some(crate::contracts::TranslationStatus::Queued),
        Some("running") => Some(crate::contracts::TranslationStatus::Running),
        Some("succeeded") => Some(crate::contracts::TranslationStatus::Succeeded),
        Some("failed") => Some(crate::contracts::TranslationStatus::Failed),
        Some("skipped") => Some(crate::contracts::TranslationStatus::Skipped),
        Some("quota_exceeded") => Some(crate::contracts::TranslationStatus::QuotaExceeded),
        _ => None,
    }
}

pub(super) fn keyword_terms(query: &str) -> Vec<String> {
    query
        .split(|ch: char| ch.is_whitespace() || ch.is_ascii_punctuation())
        .map(str::trim)
        .filter(|term| !term.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

pub(super) fn library_file_id(metadata_json: &Value) -> Option<Uuid> {
    metadata_json
        .get("library_file_id")
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
}

pub(super) fn library_section_label(metadata_json: &Value) -> Option<String> {
    metadata_json
        .get("library_section_label")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

pub(super) fn library_path(metadata_json: &Value) -> Option<String> {
    metadata_json
        .get("library_path")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}
