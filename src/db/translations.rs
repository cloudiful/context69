use anyhow::Result;
use context69_contracts::{DocumentChunkResponse, DocumentResponse, TranslationStatus};
use sqlx::FromRow;
use uuid::Uuid;

use super::Database;
use crate::domain::AccessScope;

#[derive(Debug, FromRow)]
struct TranslationVersionRow {
    id: Uuid,
    target_locale: String,
    translated_title: String,
    translated_summary: Option<String>,
}

#[derive(Debug, FromRow)]
struct TranslationChunkRow {
    id: Uuid,
    chunk_index: i32,
    chunk_text: String,
}

#[derive(Debug, FromRow)]
struct TranslationStatusRow {
    status: String,
    source_locale: Option<String>,
}

impl Database {
    pub async fn get_document_localized(
        &self,
        document_id: i64,
        locale: Option<&str>,
        scope: &AccessScope,
    ) -> Result<Option<DocumentResponse>> {
        let Some(mut document) = self.get_document(document_id, scope).await? else {
            return Ok(None);
        };
        let Some(locale) = locale else {
            document.content_locale = Some("original".to_string());
            return Ok(Some(document));
        };
        document.requested_locale = Some(locale.to_string());
        let version = sqlx::query_file_as!(
            TranslationVersionRow,
            "src/sql/db/translations/get_current_version.sql",
            document_id,
            locale
        )
        .fetch_optional(&self.pool)
        .await?;
        if let Some(version) = version {
            let chunks = sqlx::query_file_as!(
                TranslationChunkRow,
                "src/sql/db/translations/get_version_chunks.sql",
                version.id
            )
            .fetch_all(&self.pool)
            .await?;
            document.title = version.translated_title;
            document.summary = version.translated_summary;
            document.chunks = chunks
                .into_iter()
                .map(|chunk| DocumentChunkResponse {
                    chunk_id: chunk.id,
                    chunk_index: chunk.chunk_index,
                    text: chunk.chunk_text,
                })
                .collect();
            document.content_locale = Some(version.target_locale);
            document.translation_status = Some(TranslationStatus::Succeeded);
            document.is_fallback = false;
            return Ok(Some(document));
        }
        let status = sqlx::query_file_as!(
            TranslationStatusRow,
            "src/sql/db/translations/get_latest_status.sql",
            document_id,
            locale
        )
        .fetch_optional(&self.pool)
        .await?;
        document.content_locale = status
            .as_ref()
            .and_then(|value| value.source_locale.clone())
            .or_else(|| Some("original".to_string()));
        document.translation_status =
            Some(match status.as_ref().map(|value| value.status.as_str()) {
                Some("queued") => TranslationStatus::Queued,
                Some("running") => TranslationStatus::Running,
                Some("failed") => TranslationStatus::Failed,
                Some("skipped") => TranslationStatus::Skipped,
                Some("quota_exceeded") => TranslationStatus::QuotaExceeded,
                Some("succeeded") => TranslationStatus::Succeeded,
                _ => TranslationStatus::Unavailable,
            });
        document.is_fallback = true;
        Ok(Some(document))
    }
}
