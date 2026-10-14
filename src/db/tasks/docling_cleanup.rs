//! Durable owned Docling remote cleanup outbox (issue #84 P2).
//!
//! Local cancellation terminalizes immediately (204) but must not abandon the
//! fork remote: the same transaction that flips the ledger enqueues an
//! immutable cleanup request (remote id + submit-time endpoint snapshot).
//! The background worker claims due rows with `FOR UPDATE SKIP LOCKED`,
//! cancels queued remotes and observes started remotes to terminal, never
//! fetching results and never occupying a local processing slot.
//!
//! No foreign-key cascade: rows survive task/item purge so a deleted task's
//! remote is still terminated or visibly terminal. Identity is
//! (endpoint, remote id); legacy NULL-endpoint rows are idempotent by job id.
//! Endpoints never reach API responses; only safe status fields are projected.

use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

use crate::db::Database;

/// One cleanup outbox row.
#[derive(Debug, Clone, FromRow)]
pub struct StoredDoclingCleanup {
    pub id: Uuid,
    pub remote_job_id: Uuid,
    pub task_id: Uuid,
    pub item_id: Uuid,
    pub remote_task_id: String,
    pub endpoint_base_url: Option<String>,
    pub reason: String,
    pub status: String,
    pub outcome: Option<String>,
    pub attempt_count: i32,
    pub next_attempt_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub cleanup_deadline_at: DateTime<Utc>,
    pub claim_token: Option<Uuid>,
    pub claim_until: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
}

/// Safe projection for the admin list (no endpoint, no raw remote body).
#[derive(Debug, Clone, FromRow)]
pub struct DoclingCleanupView {
    pub id: Uuid,
    pub remote_job_id: Uuid,
    pub task_id: Uuid,
    pub item_id: Uuid,
    pub remote_task_id: String,
    pub reason: String,
    pub status: String,
    pub outcome: Option<String>,
    pub attempt_count: i32,
    pub next_attempt_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub cleanup_deadline_at: DateTime<Utc>,
    pub last_error: Option<String>,
}

impl Database {
    /// Claims up to `limit` due rows with a 60s lease under `claim_token`.
    pub async fn claim_docling_cleanup(
        &self,
        limit: i64,
        claim_token: Uuid,
    ) -> Result<Vec<StoredDoclingCleanup>> {
        Ok(sqlx::query_file_as!(
            StoredDoclingCleanup,
            "src/sql/db/tasks/docling_remote_cleanup/claim_due.sql",
            limit,
            claim_token,
        )
        .fetch_all(self.pool())
        .await?)
    }

    /// Settles one claimed row with CAS on the claim token. Returns whether
    /// the settle committed (false means a stale claimer lost the race).
    pub async fn settle_docling_cleanup(
        &self,
        id: Uuid,
        status: &str,
        outcome: Option<&str>,
        last_error: Option<&str>,
        next_attempt_at: DateTime<Utc>,
        claim_token: Uuid,
    ) -> Result<bool> {
        Ok(sqlx::query_file_scalar!(
            "src/sql/db/tasks/docling_remote_cleanup/settle.sql",
            id,
            status,
            outcome,
            last_error,
            next_attempt_at,
            claim_token,
        )
        .fetch_optional(self.pool())
        .await?
        .is_some())
    }

    pub async fn docling_cleanup_claim_is_detached(&self, id: Uuid, token: Uuid) -> Result<bool> {
        Ok(sqlx::query_file_scalar!(
            "src/sql/db/tasks/docling_remote_cleanup/check_claim.sql",
            id,
            token,
        )
        .fetch_one(self.pool())
        .await?)
    }

    /// Counts rows currently due for remote work.
    pub async fn count_due_docling_cleanup(&self) -> Result<i64> {
        Ok(
            sqlx::query_file_scalar!("src/sql/db/tasks/docling_remote_cleanup/count_due.sql",)
                .fetch_one(self.pool())
                .await?,
        )
    }

    /// Bounded recent rows for the admin list (safe projection, newest first).
    pub async fn list_recent_docling_cleanup(&self, limit: i64) -> Result<Vec<DoclingCleanupView>> {
        Ok(sqlx::query_file_as!(
            DoclingCleanupView,
            "src/sql/db/tasks/docling_remote_cleanup/list_recent.sql",
            limit,
        )
        .fetch_all(self.pool())
        .await?)
    }

    /// Direct enqueue for a submitted-but-unpersisted orphan (synthetic job id).
    /// Idempotent; returns whether a row was inserted.
    pub async fn enqueue_direct_docling_cleanup(
        &self,
        remote_job_id: Uuid,
        task_id: Uuid,
        item_id: Uuid,
        remote_task_id: &str,
        endpoint_base_url: Option<&str>,
        reason: &str,
    ) -> Result<bool> {
        Ok(sqlx::query_file_scalar!(
            "src/sql/db/tasks/docling_remote_cleanup/enqueue_direct.sql",
            remote_job_id,
            task_id,
            item_id,
            remote_task_id,
            endpoint_base_url,
            reason,
        )
        .fetch_optional(self.pool())
        .await?
        .is_some())
    }

    /// Detaches an explicitly unused exact job only while the item lease owns it.
    pub async fn detach_unused_owned_docling_job(
        &self,
        job_id: Uuid,
        item_id: Uuid,
        item_lease: Uuid,
    ) -> Result<bool> {
        Ok(sqlx::query_file_scalar!(
            "src/sql/db/tasks/docling_remote_cleanup/detach_unused_owned_job.sql",
            job_id,
            item_id,
            item_lease,
        )
        .fetch_optional(self.pool())
        .await?
        .is_some())
    }

    /// Admin kick: wakes eligible owned rows due in the future. Returns woken ids.
    pub async fn wake_due_docling_cleanup(&self) -> Result<Vec<Uuid>> {
        Ok(
            sqlx::query_file_scalar!("src/sql/db/tasks/docling_remote_cleanup/wake_due.sql",)
                .fetch_all(self.pool())
                .await?,
        )
    }
}
