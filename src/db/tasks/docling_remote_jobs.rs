use anyhow::Result;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

use crate::db::Database;

/// Durable Docling remote-job row (issue 650 P3).
///
/// The row is a crash-recovery reference for the blocking worker flow: the
/// worker submits, persists the remote id, then polls and fetches inline
/// while holding its item lease. There is no independent poll lease and no
/// poll sweep; only the worker that owns the item lease may commit the
/// success transition, and recovery cancels rows whose owner is gone.
/// `endpoint_base_url` is the immutable submit-time fork API base
/// (`.../v1`); NULL marks a pre-P2 legacy row that never issues HTTP.
#[derive(Debug, Clone, FromRow)]
pub struct StoredDoclingRemoteJob {
    pub id: Uuid,
    pub task_id: Uuid,
    pub item_id: Uuid,
    pub provider: String,
    /// The remote id is unknown while the row is `submitting` and may stay
    /// unknown when a dispatch outcome could not be resolved.
    pub remote_task_id: Option<String>,
    pub status: String,
    pub remote_status: Option<String>,
    /// Consecutive transport-error count, not the item's claim ordinal.
    pub attempt_count: i32,
    /// Normal probe cadence step (`0` before the first pending observation).
    pub poll_step: i32,
    pub next_poll_at: DateTime<Utc>,
    /// The database-clock instant the row was loaded, never client time.
    pub observed_at: DateTime<Utc>,
    pub last_polled_at: Option<DateTime<Utc>>,
    pub deadline_at: Option<DateTime<Utc>>,
    pub endpoint_base_url: Option<String>,
    pub lease_token: Option<Uuid>,
    pub lease_until: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    /// Whether the prepared send was invoked. Diagnostic only: neither value
    /// authorizes an automatic retry.
    pub dispatch_attempted: bool,
    /// The captured execution that created the durable intent.
    pub submission_execution_task_id: Option<Uuid>,
    /// The execution generation of the durable intent.
    pub submission_generation: Option<i64>,
    /// The frozen, non-secret dispatch identity persisted before any send.
    pub submission_identity: Value,
}

/// One fail-closed Docling submission of a task.
#[derive(Debug, Clone, FromRow)]
pub struct UnresolvedDoclingSubmission {
    pub id: Uuid,
    pub task_id: Uuid,
    pub item_id: Uuid,
    pub status: String,
    pub dispatch_attempted: bool,
}

/// One submission that must be parked as a durable unknown outcome.
#[derive(Debug, Clone)]
pub struct ParkSubmissionUncertainRequest<'a> {
    pub task_id: Uuid,
    pub item_id: Uuid,
    /// The captured execution of the item. A request without it is refused.
    pub execution: super::ExecutionContext,
    pub attempt_id: i64,
    pub job_id: Uuid,
    /// A fixed, sanitized reason that never carries an endpoint, a credential
    /// or a response body.
    pub reason: &'a str,
}

/// Explicit consent for archiving a settled unknown Docling submission.
///
/// It is internal to one transaction-owning task operation; there is no
/// standalone committing acknowledgement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DoclingRiskAck {
    /// No acknowledgement: an unresolved submission refuses the operation.
    #[default]
    Unacknowledged,
    /// The caller explicitly acknowledged the duplicate risk.
    Acknowledged,
}

/// One locked unacknowledged submission with its item settlement state.
#[derive(Debug, Clone, FromRow)]
pub struct LockedDoclingUncertainty {
    pub job_id: Uuid,
    pub item_id: Uuid,
    pub job_status: String,
    pub item_status: String,
    pub item_waiting_reason: Option<String>,
    pub item_lease_token: Option<Uuid>,
    pub item_lease_until: Option<DateTime<Utc>>,
}

/// Safe diagnose projection of one item's latest Docling submission.
#[derive(Debug, Clone, FromRow)]
pub struct StoredDoclingSubmissionView {
    pub item_id: Uuid,
    pub status: String,
    pub dispatch_attempted: bool,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    /// The item state the submission was found in; only the prescan reads it.
    pub item_status: String,
    pub item_waiting_reason: Option<String>,
    pub item_lease_token: Option<Uuid>,
    pub item_lease_until: Option<DateTime<Utc>>,
}

/// One committed uncertain park.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoclingSubmissionOutcome {
    /// The unknown outcome was persisted and the item parked without a due time.
    Parked,
    /// The library actually cancelled the segment; the domain followed it.
    Cancelled,
    /// The captured lease is gone; nothing was written.
    LeaseLost,
    /// A guarded write matched nothing; nothing was written.
    Stale,
}

/// The captured execution and identity of one item submission.
#[derive(Debug, Clone)]
pub struct DoclingSubmissionFence {
    pub task_id: Uuid,
    pub item_id: Uuid,
    pub lease_token: Uuid,
    /// The library task of the captured attempt.
    pub execution_task_id: Uuid,
    /// The execution generation of the captured attempt.
    pub generation: i64,
}

/// One durable dispatch intent persisted before any HTTP send.
#[derive(Debug, Clone)]
pub struct DoclingSubmissionIntent<'a> {
    pub fence: DoclingSubmissionFence,
    /// Normalized, non-secret endpoint identity captured at submit time.
    pub endpoint_base_url: &'a str,
    /// Whitelisted frozen identity of the dispatch.
    pub submission_identity: &'a Value,
    pub deadline_at: DateTime<Utc>,
}

impl Database {
    pub async fn create_docling_remote_job(
        &self,
        task_id: Uuid,
        item_id: Uuid,
        remote_task_id: &str,
        remote_status: Option<&str>,
        next_poll_at: Option<DateTime<Utc>>,
        deadline_at: Option<DateTime<Utc>>,
        endpoint_base_url: Option<&str>,
    ) -> Result<StoredDoclingRemoteJob> {
        Ok(sqlx::query_file_as!(
            StoredDoclingRemoteJob,
            "src/sql/db/tasks/docling_remote_jobs/create.sql",
            task_id,
            item_id,
            remote_task_id,
            remote_status,
            next_poll_at,
            deadline_at,
            endpoint_base_url,
        )
        .fetch_one(self.pool())
        .await?)
    }

    pub async fn get_active_docling_remote_job_for_item(
        &self,
        item_id: Uuid,
    ) -> Result<Option<StoredDoclingRemoteJob>> {
        Ok(sqlx::query_file_as!(
            StoredDoclingRemoteJob,
            "src/sql/db/tasks/docling_remote_jobs/get_active_by_item.sql",
            item_id
        )
        .fetch_optional(self.pool())
        .await?)
    }

    pub async fn cancel_active_docling_remote_job_for_item(
        &self,
        item_id: Uuid,
        last_error: Option<&str>,
    ) -> Result<Option<StoredDoclingRemoteJob>> {
        Ok(sqlx::query_file_as!(
            StoredDoclingRemoteJob,
            "src/sql/db/tasks/docling_remote_jobs/cancel_active_for_item.sql",
            item_id,
            last_error
        )
        .fetch_optional(self.pool())
        .await?)
    }

    /// Persists one durable dispatch intent before any HTTP send.
    ///
    /// `None` means the item fence refused or the unresolved unique index
    /// already holds a row; the caller rereads that row and never sends a
    /// second request.
    pub async fn begin_docling_submission(
        &self,
        intent: DoclingSubmissionIntent<'_>,
    ) -> Result<Option<StoredDoclingRemoteJob>> {
        Ok(sqlx::query_file_as!(
            StoredDoclingRemoteJob,
            "src/sql/db/tasks/docling_remote_jobs/begin_docling_submission.sql",
            intent.fence.item_id,
            intent.fence.task_id,
            intent.fence.lease_token,
            intent.fence.execution_task_id,
            intent.fence.generation,
            intent.endpoint_base_url,
            intent.submission_identity,
            intent.deadline_at,
        )
        .fetch_optional(self.pool())
        .await?)
    }

    /// Durably records that the prepared send is about to be invoked.
    pub async fn mark_docling_dispatch_attempted(
        &self,
        job_id: Uuid,
        fence: &DoclingSubmissionFence,
    ) -> Result<bool> {
        let updated: Option<Uuid> = sqlx::query_file_scalar!(
            "src/sql/db/tasks/docling_remote_jobs/mark_dispatch_attempted.sql",
            job_id,
            fence.item_id,
            fence.task_id,
            fence.lease_token,
            fence.execution_task_id,
            fence.generation,
        )
        .fetch_optional(self.pool())
        .await?;
        Ok(updated.is_some())
    }

    /// Confirms one accepted submission's known remote id.
    pub async fn confirm_docling_submission(
        &self,
        job_id: Uuid,
        fence: &DoclingSubmissionFence,
        remote_task_id: &str,
        next_poll_at: DateTime<Utc>,
    ) -> Result<Option<StoredDoclingRemoteJob>> {
        Ok(sqlx::query_file_as!(
            StoredDoclingRemoteJob,
            "src/sql/db/tasks/docling_remote_jobs/confirm_docling_submission.sql",
            job_id,
            fence.item_id,
            fence.task_id,
            fence.lease_token,
            fence.execution_task_id,
            fence.generation,
            remote_task_id,
            next_poll_at,
        )
        .fetch_optional(self.pool())
        .await?)
    }

    /// Closes one not-dispatched intent after a preparation failure.
    pub async fn mark_docling_preparation_failed(
        &self,
        job_id: Uuid,
        fence: &DoclingSubmissionFence,
        reason: &str,
    ) -> Result<bool> {
        let updated: Option<Uuid> = sqlx::query_file_scalar!(
            "src/sql/db/tasks/docling_remote_jobs/mark_preparation_failed.sql",
            job_id,
            fence.item_id,
            fence.task_id,
            fence.lease_token,
            fence.execution_task_id,
            fence.generation,
            reason,
        )
        .fetch_optional(self.pool())
        .await?;
        Ok(updated.is_some())
    }

    /// Parks one submission as a durable unknown outcome.
    pub async fn mark_docling_submission_uncertain(
        &self,
        job_id: Uuid,
        fence: &DoclingSubmissionFence,
        reason: Option<&str>,
    ) -> Result<bool> {
        let mut tx = self.pool().begin().await?;
        let updated = self
            .mark_docling_submission_uncertain_in(&mut tx, job_id, fence, reason)
            .await?;
        tx.commit().await?;
        Ok(updated)
    }

    /// Transaction-scoped unknown-outcome marker for the atomic park.
    pub(crate) async fn mark_docling_submission_uncertain_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        job_id: Uuid,
        fence: &DoclingSubmissionFence,
        reason: Option<&str>,
    ) -> Result<bool> {
        let updated: Option<Uuid> = sqlx::query_file_scalar!(
            "src/sql/db/tasks/docling_remote_jobs/mark_submission_uncertain.sql",
            job_id,
            fence.item_id,
            reason,
            fence.execution_task_id,
            fence.generation,
        )
        .fetch_optional(&mut **tx)
        .await?;
        Ok(updated.is_some())
    }

    /// Loads the one unresolved submission of an item.
    pub async fn get_unresolved_docling_remote_job_for_item(
        &self,
        item_id: Uuid,
    ) -> Result<Option<StoredDoclingRemoteJob>> {
        Ok(sqlx::query_file_as!(
            StoredDoclingRemoteJob,
            "src/sql/db/tasks/docling_remote_jobs/get_unresolved_by_item.sql",
            item_id
        )
        .fetch_optional(self.pool())
        .await?)
    }

    /// Lists the fail-closed submissions of a bounded task set.
    pub async fn list_unresolved_docling_submissions_for_tasks(
        &self,
        task_ids: &[Uuid],
    ) -> Result<Vec<UnresolvedDoclingSubmission>> {
        Ok(sqlx::query_file_as!(
            UnresolvedDoclingSubmission,
            "src/sql/db/tasks/docling_remote_jobs/list_unresolved_for_tasks.sql",
            task_ids,
        )
        .fetch_all(self.pool())
        .await?)
    }

    /// Validates and archives the settled unknown submissions of one task
    /// inside the caller's existing transaction.
    ///
    /// Every candidate is locked and classified before any write. A
    /// `submitting` row, an unsettled item (anything but `waiting/docling` or
    /// `cancelled` with no lease) or an unacknowledged call refuses the whole
    /// operation; only validated `submission_uncertain` rows are archived, and
    /// their item ids are returned for the operation's own reopen step.
    pub(crate) async fn ack_docling_uncertain_in(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        task_id: Uuid,
        user_id: Option<i64>,
        ack: DoclingRiskAck,
    ) -> Result<Vec<Uuid>> {
        let locked = sqlx::query_file_as!(
            LockedDoclingUncertainty,
            "src/sql/db/tasks/docling_remote_jobs/lock_uncertain_for_ack.sql",
            task_id,
            user_id,
        )
        .fetch_all(&mut **tx)
        .await?;
        if locked.is_empty() {
            return Ok(Vec::new());
        }
        if ack == DoclingRiskAck::Unacknowledged {
            return Err(crate::domain_errors::DomainError::docling_submission_uncertain(
                "a Docling submission of this task has an unknown outcome; acknowledgement is required",
            )
            .into());
        }
        for row in &locked {
            let settled = (row.item_status == "waiting"
                && row.item_waiting_reason.as_deref() == Some("docling"))
                || row.item_status == "cancelled";
            let leased = row.item_lease_token.is_some() || row.item_lease_until.is_some();
            if row.job_status == "submitting" || !settled || leased {
                return Err(
                    crate::domain_errors::DomainError::docling_submission_uncertain(
                        "a Docling submission of this task is still in flight",
                    )
                    .into(),
                );
            }
        }
        let validated: Vec<Uuid> = locked
            .iter()
            .filter(|row| row.job_status == "submission_uncertain")
            .map(|row| row.job_id)
            .collect();
        if validated.is_empty() {
            return Ok(Vec::new());
        }
        let archived: Vec<Uuid> = sqlx::query_file_scalar!(
            "src/sql/db/tasks/docling_remote_jobs/archive_uncertain_for_task.sql",
            task_id,
            &validated,
        )
        .fetch_all(&mut **tx)
        .await?;
        Ok(archived)
    }

    /// Loads the latest safe Docling submission view of each item in a bounded
    /// task set. Never returns identity, endpoint, remote id or error text.
    pub async fn list_docling_submission_views_for_tasks(
        &self,
        task_ids: &[Uuid],
    ) -> Result<Vec<StoredDoclingSubmissionView>> {
        Ok(sqlx::query_file_as!(
            StoredDoclingSubmissionView,
            "src/sql/db/tasks/docling_remote_jobs/list_views_for_tasks.sql",
            task_ids,
        )
        .fetch_all(self.pool())
        .await?)
    }

    /// Milliseconds until the next known remote job needs a probe or hits its
    /// deadline. `None` means no live known job can be woken.
    pub async fn next_docling_probe_delay(&self) -> Result<Option<i64>> {
        Ok(
            sqlx::query_file_scalar!("src/sql/db/tasks/docling_remote_jobs/next_due.sql")
                .fetch_one(self.pool())
                .await?,
        )
    }
}
