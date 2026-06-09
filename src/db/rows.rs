use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDate, Utc};
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

use crate::{
    contracts::{SearchHit, SearchMode},
    db::StoredSearchSettings,
    normalize::is_meaningful_text,
};

#[derive(Debug, Clone, FromRow)]
pub(super) struct CheckpointRow {
    pub(super) cursor_updated_at: Option<DateTime<Utc>>,
    pub(super) cursor_external_id: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub(super) struct DocumentRow {
    pub(super) id: i64,
    pub(super) group_key: String,
    pub(super) project_key: String,
    pub(super) visibility: String,
    pub(super) source_key: String,
    pub(super) external_id: String,
    pub(super) title: String,
    pub(super) summary: Option<String>,
    pub(super) source_uri: String,
    pub(super) published_at: Option<NaiveDate>,
    pub(super) updated_at_source: DateTime<Utc>,
    pub(super) record_hash: String,
    pub(super) metadata_json: Value,
}

#[derive(Debug, Clone, FromRow)]
pub(super) struct ChunkRow {
    pub(super) id: Uuid,
    pub(super) chunk_index: i32,
    pub(super) chunk_text: String,
}

#[derive(Debug, Clone, FromRow)]
pub(super) struct SearchHitRow {
    pub(super) chunk_id: Uuid,
    pub(super) document_id: i64,
    pub(super) group_key: String,
    pub(super) project_key: String,
    pub(super) visibility: String,
    pub(super) source_key: String,
    pub(super) external_id: String,
    pub(super) title: String,
    pub(super) summary: Option<String>,
    pub(super) source_uri: String,
    pub(super) published_at: Option<NaiveDate>,
    pub(super) chunk_index: i32,
    pub(super) chunk_text: String,
    pub(super) metadata_json: Value,
}

#[derive(Debug, Clone, FromRow)]
pub(super) struct ReindexChunkRow {
    pub(super) chunk_id: Uuid,
    pub(super) document_id: i64,
    pub(super) group_id: i64,
    pub(super) group_key: String,
    pub(super) project_id: i64,
    pub(super) project_key: String,
    pub(super) visibility: String,
    pub(super) source_key: String,
    pub(super) external_id: String,
    pub(super) title: String,
    pub(super) summary: Option<String>,
    pub(super) source_uri: String,
    pub(super) published_at: Option<NaiveDate>,
    pub(super) updated_at_source: DateTime<Utc>,
    pub(super) record_hash: String,
    pub(super) chunk_index: i32,
    pub(super) chunk_text: String,
    pub(super) metadata_json: Value,
}

#[derive(Debug, Clone, FromRow)]
pub(super) struct DoclingSettingsRow {
    pub(super) base_url: String,
    pub(super) timeout_secs: i64,
    pub(super) poll_interval_secs: i64,
    pub(super) pdf_backend: Option<String>,
    pub(super) images_scale: Option<f64>,
    pub(super) image_export_mode: Option<String>,
    pub(super) do_ocr: bool,
    pub(super) force_ocr: bool,
    pub(super) ocr_engine: Option<String>,
    pub(super) ocr_lang: Vec<String>,
    pub(super) do_code_enrichment: bool,
    pub(super) do_formula_enrichment: bool,
    pub(super) do_picture_description: bool,
    pub(super) provider_account_key: Option<String>,
    pub(super) vlm_pipeline_model: Option<String>,
    pub(super) picture_description_model: Option<String>,
    pub(super) code_formula_model: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub(super) struct SearchSettingsRow {
    pub(super) mode: String,
    pub(super) rerank_enabled: bool,
    pub(super) rerank_base_url: String,
    pub(super) rerank_model: String,
    pub(super) candidate_limit: i64,
    pub(super) timeout_secs: i64,
    pub(super) api_key: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub(super) struct KeywordSearchHitRow {
    pub(super) chunk_id: Uuid,
    pub(super) document_id: i64,
    pub(super) group_key: String,
    pub(super) project_key: String,
    pub(super) visibility: String,
    pub(super) source_key: String,
    pub(super) external_id: String,
    pub(super) title: String,
    pub(super) summary: Option<String>,
    pub(super) source_uri: String,
    pub(super) published_at: Option<NaiveDate>,
    pub(super) chunk_index: i32,
    pub(super) chunk_text: String,
    pub(super) metadata_json: Value,
    pub(super) keyword_score: f32,
    pub(super) match_reason: String,
}

#[derive(Debug, Clone, FromRow)]
pub(super) struct ProviderAccountRow {
    pub(super) account_key: String,
    pub(super) provider_kind: String,
    pub(super) display_name: String,
    pub(super) base_url: String,
    pub(super) api_key: Option<String>,
    pub(super) disabled_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, FromRow)]
pub(super) struct RuntimeQdrantSettingsRow {
    pub(super) url: String,
    pub(super) collection_name: String,
    pub(super) recreate_on_dimension_mismatch: bool,
}

#[derive(Debug, Clone, FromRow)]
pub(super) struct RuntimeEmbeddingSettingsRow {
    pub(super) provider_account_key: String,
    pub(super) model: String,
    pub(super) dimensions: i64,
    pub(super) timeout_secs: i64,
}

#[derive(Debug, Clone, FromRow)]
pub(super) struct RuntimeSchedulerSettingsRow {
    pub(super) interval_secs: i64,
    pub(super) run_on_start: bool,
    pub(super) max_concurrency: i64,
    pub(super) job_id: String,
    pub(super) valkey_url: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub(super) struct RuntimeChunkingSettingsRow {
    pub(super) max_chars: i64,
    pub(super) overlap_chars: i64,
}

#[derive(Debug, Clone, FromRow)]
pub(super) struct RuntimeFileLibrarySettingsRow {
    pub(super) storage_root: String,
    pub(super) max_upload_size_mb: i64,
    pub(super) max_upload_request_size_mb: i64,
    pub(super) ingest_concurrency: i64,
    pub(super) pdf_pages_per_task: i64,
}

#[derive(Debug, Clone, FromRow)]
pub(super) struct SourceConnectionRow {
    pub(super) name: String,
    pub(super) database_url: String,
}

#[derive(Debug, Clone, FromRow)]
pub(super) struct RerankItemScoreRow {
    pub(super) rerank_model: String,
    pub(super) query_hash: String,
    pub(super) query_text_trimmed: String,
    pub(super) chunk_id: Uuid,
    pub(super) score: f32,
}

#[derive(Debug, Clone, FromRow)]
pub(super) struct ExistingDocumentRow {
    pub(super) id: i64,
    pub(super) record_hash: String,
}

#[derive(Debug, Clone, FromRow)]
pub(super) struct CheckpointWithKeyRow {
    pub(super) source_key: String,
    pub(super) cursor_updated_at: Option<DateTime<Utc>>,
    pub(super) cursor_external_id: Option<String>,
    pub(super) last_success_at: Option<DateTime<Utc>>,
}

pub(super) fn search_settings_from_row(row: SearchSettingsRow) -> Result<StoredSearchSettings> {
    let mode = match row.mode.as_str() {
        "vector" => SearchMode::Vector,
        "hybrid" => SearchMode::Hybrid,
        other => return Err(anyhow::anyhow!("unsupported search mode: {other}")),
    };

    Ok(StoredSearchSettings {
        mode,
        rerank_enabled: row.rerank_enabled,
        rerank_base_url: row.rerank_base_url,
        rerank_model: row.rerank_model,
        candidate_limit: usize::try_from(row.candidate_limit)
            .context("search candidate_limit must be non-negative")?,
        timeout_secs: u64::try_from(row.timeout_secs)
            .context("search timeout_secs must be non-negative")?,
        api_key: row.api_key,
    })
}

pub(super) fn is_library_file(metadata_json: &Value) -> bool {
    metadata_json
        .get("is_library_file")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

pub(super) fn search_hit_from_keyword_row(row: KeywordSearchHitRow) -> SearchHit {
    SearchHit {
        chunk_id: row.chunk_id,
        document_id: row.document_id,
        group_key: row.group_key,
        project_key: row.project_key,
        visibility: row.visibility.parse().unwrap_or(crate::contracts::Visibility::Private),
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
