use anyhow::Result;
use chrono::{DateTime, Utc};
use context69_contracts::ExtractionHealthResponse;
use sqlx::FromRow;
use uuid::Uuid;

use super::{ExtractionJobRecord, ExtractionStore, ExtractionVersionInput, ExtractionVersionRow};

pub(crate) struct FinishExtractionJob<'a> {
    pub id: Uuid,
    pub status: &'a str,
    pub provider_key: Option<&'a str>,
    pub provider_config_hash: Option<&'a str>,
    pub error_message: Option<&'a str>,
    pub failure_class: Option<&'a str>,
    pub next_attempt_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, FromRow)]
struct HealthRow {
    queued: Option<i64>,
    running: Option<i64>,
    awaiting_retry: Option<i64>,
    next_retry_at: Option<DateTime<Utc>>,
    failed_last_hour: Option<i64>,
    failure_class_counts: Option<serde_json::Value>,
}

pub(crate) struct ExtractionAttempt<'a> {
    pub job_id: Uuid,
    pub provider_key: &'a str,
    pub provider_config_hash: &'a str,
    pub attempt_number: i32,
    pub status: &'a str,
    pub latency_ms: i64,
    pub error_message: Option<&'a str>,
}

impl ExtractionStore {
    pub async fn job_in_group(
        &self,
        group_id: i64,
        id: Uuid,
    ) -> Result<Option<ExtractionJobRecord>> {
        Ok(sqlx::query_file_as!(
            ExtractionJobRecord,
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
    ) -> Result<Vec<ExtractionJobRecord>> {
        Ok(sqlx::query_file_as!(
            ExtractionJobRecord,
            "sql/jobs/list_for_document.sql",
            document_id,
            group_id
        )
        .fetch_all(self.pool())
        .await?)
    }

    pub async fn results_for_document(
        &self,
        document_id: i64,
    ) -> Result<Vec<ExtractionVersionRow>> {
        Ok(sqlx::query_file_as!(
            ExtractionVersionRow,
            "sql/versions/list_for_document.sql",
            document_id
        )
        .fetch_all(self.pool())
        .await?)
    }

    pub async fn retry_job(&self, group_id: i64, id: Uuid) -> Result<Option<ExtractionJobRecord>> {
        Ok(
            sqlx::query_file_as!(ExtractionJobRecord, "sql/jobs/retry.sql", id, group_id)
                .fetch_optional(self.pool())
                .await?,
        )
    }

    pub async fn insert_job(
        &self,
        document_id: i64,
        template: &super::StoredExtractionTemplate,
        record_hash: &str,
        parameters: &serde_json::Value,
    ) -> Result<ExtractionJobRecord> {
        Ok(sqlx::query_file_as!(
            ExtractionJobRecord,
            "sql/jobs/insert.sql",
            Uuid::new_v4(),
            document_id,
            template.template_key,
            template.version,
            record_hash,
            parameters
        )
        .fetch_one(self.pool())
        .await?)
    }

    pub async fn pending_ids(&self) -> Result<Vec<Uuid>> {
        Ok(sqlx::query_file_scalar!("sql/jobs/list_pending.sql")
            .fetch_all(self.pool())
            .await?)
    }

    pub async fn next_pending_at(&self) -> Result<Option<DateTime<Utc>>> {
        Ok(sqlx::query_file_scalar!("sql/next_pending_at.sql")
            .fetch_one(self.pool())
            .await?)
    }

    pub async fn health(&self) -> Result<ExtractionHealthResponse> {
        let row = sqlx::query_file_as!(HealthRow, "sql/health.sql")
            .fetch_one(self.pool())
            .await?;
        let failure_class_counts = match row
            .failure_class_counts
            .unwrap_or(serde_json::Value::Object(Default::default()))
        {
            serde_json::Value::Object(map) => map
                .into_iter()
                .filter_map(|(k, v)| v.as_i64().map(|cnt| (k, cnt)))
                .collect(),
            _ => std::collections::BTreeMap::new(),
        };
        Ok(ExtractionHealthResponse {
            queued: row.queued.unwrap_or(0),
            running: row.running.unwrap_or(0),
            awaiting_retry: row.awaiting_retry.unwrap_or(0),
            next_retry_at: row.next_retry_at,
            failed_last_hour: row.failed_last_hour.unwrap_or(0),
            failure_class_counts,
        })
    }

    pub async fn reset_interrupted(&self) -> Result<()> {
        sqlx::query_file!("sql/jobs/reset_interrupted.sql")
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn claim_job(&self, id: Uuid) -> Result<Option<ExtractionJobRecord>> {
        Ok(
            sqlx::query_file_as!(ExtractionJobRecord, "sql/jobs/claim.sql", id)
                .fetch_optional(self.pool())
                .await?,
        )
    }

    pub(crate) async fn finish_job(
        &self,
        result: FinishExtractionJob<'_>,
    ) -> Result<ExtractionJobRecord> {
        Ok(sqlx::query_file_as!(
            ExtractionJobRecord,
            "sql/jobs/finish.sql",
            result.id,
            result.status,
            result.provider_key,
            result.provider_config_hash,
            result.error_message,
            result.failure_class,
            result.next_attempt_at
        )
        .fetch_one(self.pool())
        .await?)
    }

    pub(crate) async fn release_claimed_job(&self, id: Uuid) -> Result<bool> {
        let affected = sqlx::query_file!("sql/jobs/release.sql", id)
            .execute(self.pool())
            .await?;
        Ok(affected.rows_affected() > 0)
    }

    pub(crate) async fn insert_attempt(&self, attempt: ExtractionAttempt<'_>) -> Result<()> {
        sqlx::query_file!(
            "sql/jobs/insert_attempt.sql",
            attempt.job_id,
            attempt.provider_key,
            attempt.provider_config_hash,
            attempt.attempt_number,
            attempt.status,
            attempt.latency_ms,
            attempt.error_message
        )
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn publish_version(&self, version: &ExtractionVersionInput<'_>) -> Result<()> {
        let mut tx = self.pool().begin().await?;
        sqlx::query_file!(
            "sql/versions/delete_current.sql",
            version.document_id,
            version.template_key,
            version.template_version,
            version.source_record_hash
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query_file!(
            "sql/versions/insert.sql",
            version.id,
            version.document_id,
            version.template_key,
            version.template_version,
            version.source_record_hash,
            version.provider_key,
            version.provider_config_hash,
            version.model_name,
            version.result_json
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }
}
