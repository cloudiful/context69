use anyhow::Result;
use uuid::Uuid;

use super::{TranslationJobRecord, TranslationStore, TranslationVersionInput};

pub(crate) struct FinishJob<'a> {
    pub id: Uuid,
    pub status: &'a str,
    pub source_locale: Option<&'a str>,
    pub provider_key: Option<&'a str>,
    pub provider_config_hash: Option<&'a str>,
    pub character_count: i64,
    pub error_message: Option<&'a str>,
}

pub(crate) struct TranslationAttempt<'a> {
    pub job_id: Uuid,
    pub provider_key: &'a str,
    pub provider_config_hash: &'a str,
    pub attempt_number: i32,
    pub status: &'a str,
    pub character_count: i64,
    pub latency_ms: i64,
    pub error_message: Option<&'a str>,
}

impl TranslationStore {
    pub async fn job_in_group(
        &self,
        group_id: i64,
        id: Uuid,
    ) -> Result<Option<TranslationJobRecord>> {
        Ok(sqlx::query_file_as!(
            TranslationJobRecord,
            "sql/jobs/get_in_group.sql",
            id,
            group_id
        )
        .fetch_optional(self.pool())
        .await?)
    }

    pub async fn jobs_for_document(
        &self,
        group_id: i64,
        document_id: i64,
    ) -> Result<Vec<TranslationJobRecord>> {
        Ok(sqlx::query_file_as!(
            TranslationJobRecord,
            "sql/jobs/list_for_document.sql",
            document_id,
            group_id
        )
        .fetch_all(self.pool())
        .await?)
    }

    pub async fn retry_job(&self, group_id: i64, id: Uuid) -> Result<Option<TranslationJobRecord>> {
        Ok(
            sqlx::query_file_as!(TranslationJobRecord, "sql/jobs/retry.sql", id, group_id)
                .fetch_optional(self.pool())
                .await?,
        )
    }

    pub(crate) async fn finish_job(&self, result: FinishJob<'_>) -> Result<TranslationJobRecord> {
        Ok(sqlx::query_file_as!(
            TranslationJobRecord,
            "sql/jobs/finish.sql",
            result.id,
            result.status,
            result.source_locale,
            result.provider_key,
            result.provider_config_hash,
            result.character_count,
            result.error_message
        )
        .fetch_one(self.pool())
        .await?)
    }

    pub(crate) async fn release_claimed_job(&self, id: Uuid) -> Result<bool> {
        Ok(sqlx::query_file_scalar!("sql/jobs/release.sql", id)
            .fetch_optional(self.pool())
            .await?
            .is_some())
    }

    pub(crate) async fn insert_attempt(&self, attempt: TranslationAttempt<'_>) -> Result<()> {
        sqlx::query_file!(
            "sql/jobs/insert_attempt.sql",
            attempt.job_id,
            attempt.provider_key,
            attempt.provider_config_hash,
            attempt.attempt_number,
            attempt.status,
            attempt.character_count,
            attempt.latency_ms,
            attempt.error_message
        )
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn current_translation_chunk_ids(
        &self,
        document_id: i64,
        target_locale: &str,
    ) -> Result<Vec<Uuid>> {
        Ok(sqlx::query_file_scalar!(
            "sql/versions/list_chunk_ids.sql",
            document_id,
            target_locale
        )
        .fetch_all(self.pool())
        .await?)
    }

    pub async fn publish_version(
        &self,
        version: &TranslationVersionInput<'_>,
        chunks: &[(Uuid, String)],
    ) -> Result<()> {
        let mut tx = self.pool().begin().await?;
        sqlx::query_file!(
            "sql/versions/delete_current.sql",
            version.document_id,
            version.target_locale,
            version.source_record_hash
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query_file!(
            "sql/versions/insert.sql",
            version.id,
            version.document_id,
            version.target_locale,
            version.source_locale,
            version.source_record_hash,
            version.provider_key,
            version.provider_config_hash,
            version.model_name,
            version.title,
            version.summary,
            version.body_text
        )
        .execute(&mut *tx)
        .await?;
        for (index, (id, text)) in chunks.iter().enumerate() {
            sqlx::query_file!(
                "sql/versions/insert_chunk.sql",
                id,
                version.id,
                version.document_id,
                version.target_locale,
                index as i32,
                text
            )
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }
}
