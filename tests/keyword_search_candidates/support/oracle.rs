use anyhow::Result;
use context69::contracts::{SearchHit, SearchRequest, TranslationStatus, Visibility};
use context69::db::Database;
use serde_json::Value;
use sqlx::{Row, postgres::PgRow};
use uuid::Uuid;

pub const FROZEN_HEAD_SQL: &str = include_str!("../fixtures/keyword_search_head.sql");

pub async fn search(
    db: &Database,
    request: &SearchRequest,
    scope: &context69::domain::AccessScope,
    limit: usize,
) -> Result<Vec<SearchHit>> {
    let normalized_query = request.query.trim().to_lowercase();
    if normalized_query.is_empty() {
        return Ok(Vec::new());
    }
    let phrase_pattern = format!("%{normalized_query}%");
    let terms = normalized_query
        .split(|ch: char| ch.is_whitespace() || ch.is_ascii_punctuation())
        .map(str::trim)
        .filter(|term| !term.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let metadata_filters = serde_json::to_value(&request.metadata_filters)?;
    let limit = i64::try_from(limit)?;
    let rows = sqlx::query(FROZEN_HEAD_SQL)
        .bind(&normalized_query)
        .bind(&phrase_pattern)
        .bind(terms)
        .bind(request.source_key.as_deref())
        .bind(request.group_path.as_deref())
        .bind(request.published_after)
        .bind(request.published_before)
        .bind(scope.private_group_ids.clone())
        .bind(limit)
        .bind(request.locale.as_deref())
        .bind(metadata_filters)
        .fetch_all(db.pool())
        .await?;
    rows.into_iter()
        .map(|row| search_hit_from_frozen_row(row, request.locale.as_deref()))
        .collect()
}

fn search_hit_from_frozen_row(row: PgRow, requested_locale: Option<&str>) -> Result<SearchHit> {
    let metadata_json: Value = row.try_get("metadata_json!")?;
    let visibility: String = row.try_get("visibility!")?;
    let content_locale: String = row.try_get("content_locale!")?;
    let translation_status: Option<String> = row.try_get("translation_status")?;
    let summary: Option<String> = row.try_get("summary")?;
    let score: f32 = row.try_get("keyword_score!")?;
    let library_file_id = metadata_json
        .get("library_file_id")
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok());
    let library_section_label = metadata_json
        .get("library_section_label")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let library_path = metadata_json
        .get("library_path")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let is_library_file = metadata_json
        .get("is_library_file")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    Ok(SearchHit {
        chunk_id: row.try_get("chunk_id!")?,
        document_id: row.try_get("document_id!")?,
        group_key: row.try_get("group_key!")?,
        group_path: row.try_get("group_path!")?,
        visibility: visibility.parse().unwrap_or(Visibility::Private),
        source_key: row.try_get("source_key!")?,
        external_id: row.try_get("external_id!")?,
        title: row.try_get("title!")?,
        summary: summary.filter(|value| value.chars().any(char::is_alphanumeric)),
        source_uri: row.try_get("source_uri!")?,
        published_at: row.try_get("published_at")?,
        chunk_index: row.try_get("chunk_index!")?,
        chunk_text: row.try_get("chunk_text!")?,
        score,
        vector_score: None,
        keyword_score: Some(score),
        rerank_score: None,
        match_reason: Some(row.try_get("match_reason!")?),
        metadata_json,
        library_file_id,
        library_section_label,
        library_path,
        is_library_file,
        requested_locale: requested_locale.map(ToOwned::to_owned),
        content_locale: Some(content_locale.clone()),
        translation_status: map_translation_status(translation_status.as_deref()),
        is_fallback: requested_locale.is_some() && content_locale == "original",
    })
}

fn map_translation_status(value: Option<&str>) -> Option<TranslationStatus> {
    match value {
        Some("queued") => Some(TranslationStatus::Queued),
        Some("running") => Some(TranslationStatus::Running),
        Some("succeeded") => Some(TranslationStatus::Succeeded),
        Some("failed") => Some(TranslationStatus::Failed),
        Some("skipped") => Some(TranslationStatus::Skipped),
        Some("quota_exceeded") => Some(TranslationStatus::QuotaExceeded),
        _ => None,
    }
}
