//! Durable payload-mirror outbox for the embedded-enrichment retirement.
//!
//! The retirement migration enqueues one row per affected document and retired
//! payload key. This module owns only the outbox mechanics: claim one due job
//! with a fenced lease, read the exact point ids the job must converge, and
//! record completion or a sanitized retry. It never stores or logs the retired
//! payload itself.

use anyhow::Result;
use sqlx::FromRow;
use uuid::Uuid;

use super::Database;

/// One claimed outbox row. `lease_token` fences completion and failure writes.
#[derive(Debug, Clone)]
pub struct ClaimedMetadataPayloadSyncJob {
    pub id: i64,
    pub document_id: i64,
    pub payload_key: String,
    pub attempts: i32,
    pub lease_token: Uuid,
}

#[derive(FromRow)]
struct ClaimedMetadataPayloadSyncJobRow {
    id: i64,
    document_id: i64,
    payload_key: String,
    attempts: i32,
    lease_token: Option<Uuid>,
}

/// Exact point ids for one document, read in one statement so the source hash
/// and the enumerated generations belong to the same snapshot.
#[derive(Debug, Clone)]
pub struct MetadataPayloadSyncPoints {
    pub record_hash: String,
    pub originals: Vec<Uuid>,
    pub translations: Vec<Uuid>,
}

#[derive(FromRow)]
struct MetadataPayloadSyncPointsRow {
    record_hash: String,
    originals: Option<Vec<Uuid>>,
    translations: Option<Vec<Uuid>>,
}

impl Database {
    /// Claim at most one due job with `SKIP LOCKED`, leasing it for
    /// `lease_seconds`. An expired lease is claimable again; the previous token
    /// can no longer complete or fail the row.
    pub async fn claim_metadata_payload_sync(
        &self,
        lease_token: Uuid,
        lease_seconds: i32,
    ) -> Result<Option<ClaimedMetadataPayloadSyncJob>> {
        let job = sqlx::query_file_as!(
            ClaimedMetadataPayloadSyncJobRow,
            "src/sql/db/metadata_payload_sync/claim.sql",
            lease_token,
            lease_seconds
        )
        .fetch_optional(&self.pool)
        .await?;
        job.map(|row| {
            let returned_lease = row
                .lease_token
                .ok_or_else(|| anyhow::anyhow!("claimed metadata payload sync job has no lease"))?;
            Ok(ClaimedMetadataPayloadSyncJob {
                id: row.id,
                document_id: row.document_id,
                payload_key: row.payload_key,
                attempts: row.attempts,
                lease_token: returned_lease,
            })
        })
        .transpose()
    }

    /// Mark a job complete only while `lease_token` still owns it.
    pub async fn complete_metadata_payload_sync(&self, id: i64, lease_token: Uuid) -> Result<bool> {
        let result = sqlx::query_file!(
            "src/sql/db/metadata_payload_sync/complete.sql",
            id,
            lease_token
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    /// Record one failed attempt with a bounded reason and retry delay, only
    /// while `lease_token` still owns the row. The payload key stays pending.
    pub async fn fail_metadata_payload_sync(
        &self,
        id: i64,
        lease_token: Uuid,
        retry_after_seconds: i32,
        reason: &str,
    ) -> Result<bool> {
        let result = sqlx::query_file!(
            "src/sql/db/metadata_payload_sync/fail.sql",
            id,
            lease_token,
            retry_after_seconds,
            reason
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    /// Pending jobs, for startup observability only.
    pub async fn pending_metadata_payload_sync(&self) -> Result<i64> {
        let count = sqlx::query_file_scalar!("src/sql/db/metadata_payload_sync/pending_count.sql")
            .fetch_one(&self.pool)
            .await?;
        Ok(count)
    }

    /// Read the current source hash plus every original and translated point id
    /// for one document. `None` means the document is gone.
    pub async fn metadata_payload_sync_points(
        &self,
        document_id: i64,
    ) -> Result<Option<MetadataPayloadSyncPoints>> {
        let points = sqlx::query_file_as!(
            MetadataPayloadSyncPointsRow,
            "src/sql/db/metadata_payload_sync/document_points.sql",
            document_id
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(points.map(|row| MetadataPayloadSyncPoints {
            record_hash: row.record_hash,
            originals: row.originals.unwrap_or_default(),
            translations: row.translations.unwrap_or_default(),
        }))
    }
}
