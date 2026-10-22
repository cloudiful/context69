use anyhow::Result;
use chrono::DateTime;
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

use crate::db::Database;

/// One active remote job with its owning item/task state for the recovery
/// pass. `adopt_payload` carries the item payload only for legacy
/// `waiting/docling` parks (the one case that requeues with unchanged
/// content); every other row returns NULL so the periodic pass never drags
/// conversion payloads through the listing.
#[derive(Debug, Clone, FromRow)]
pub struct DoclingRecoveryRow {
    pub id: Uuid,
    pub task_id: Uuid,
    pub item_id: Uuid,
    pub remote_task_id: String,
    pub endpoint_base_url: Option<String>,
    pub deadline_at: Option<DateTime<chrono::Utc>>,
    pub item_status: String,
    pub item_waiting_reason: Option<String>,
    pub item_lease_until: Option<DateTime<chrono::Utc>>,
    pub task_status: String,
    pub adopt_payload: Option<Value>,
}

/// Per-run recovery outcome for logs. No payloads or secrets.
#[derive(Debug, Clone, Default)]
pub struct DoclingRecoverySummary {
    pub cancelled_remote_jobs: u64,
    pub adopted_items: u64,
}

fn is_live_status(status: &str) -> bool {
    matches!(status, "queued" | "running" | "waiting")
}

fn is_docling_park(row: &DoclingRecoveryRow) -> bool {
    row.item_status == "waiting" && row.item_waiting_reason.as_deref() == Some("docling")
}

impl Database {
    /// Commits one inline Docling success inside the guarded write of the item.
    ///
    /// The library attempt is locked first (a renewal, the same order the
    /// heartbeat takes), the item mirror takes the returned deadline, and then
    /// the remote job and the fetched sections are written on the same
    /// transaction. A false fence - an expired lease, a cancelled item or a
    /// superseded generation - rolls every one of those writes back, so no
    /// produced output is ever committed by a worker that lost its authority.
    pub async fn finish_docling_remote_job_with_sections(
        &self,
        job_id: Uuid,
        item_id: Uuid,
        execution: &crate::db::ExecutionContext,
        sections_payload: &Value,
        remote_status: &str,
    ) -> Result<Option<Uuid>> {
        let mut transaction = self.pool().begin().await?;
        let queue = self.execution_queue().await?;
        let lease_until = match queue.renew_in(&mut transaction, &execution.attempt).await {
            Ok(lease_until) => lease_until,
            Err(cloudiful_tasks::Error::LeaseLost { .. })
            | Err(cloudiful_tasks::Error::InvalidTaskState { .. }) => {
                transaction.rollback().await.ok();
                return Ok(None);
            }
            Err(error) => return Err(error.into()),
        };
        let still_owned = sqlx::query_file!(
            "src/sql/db/tasks/heartbeat_item.sql",
            item_id,
            execution.fencing_token(),
            lease_until,
            execution.task_id(),
            execution.generation
        )
        .execute(&mut *transaction)
        .await?
        .rows_affected()
            > 0;
        if !still_owned {
            transaction.rollback().await.ok();
            return Ok(None);
        }
        let committed = sqlx::query_file_scalar!(
            "src/sql/db/tasks/docling_remote_jobs/finish_with_sections.sql",
            job_id,
            item_id,
            sections_payload,
            remote_status,
            execution.fencing_token(),
            execution.task_id(),
            execution.generation,
        )
        .fetch_optional(&mut *transaction)
        .await?;
        let Some(committed) = committed else {
            transaction.rollback().await.ok();
            return Ok(None);
        };
        transaction.commit().await?;
        Ok(Some(committed))
    }

    /// Requeues one legacy parked Docling item with unchanged content so the
    /// next claim adopts it. Fenced on `waiting/docling`: a concurrent
    /// cancel/retry that already moved the row matches zero rows. Returns
    /// whether the item was requeued.
    pub async fn requeue_docling_waiting_item(
        &self,
        item_id: Uuid,
        payload: &Value,
    ) -> Result<bool> {
        Ok(sqlx::query_file!(
            "src/sql/db/tasks/docling_remote_jobs/requeue_waiting_item.sql",
            item_id,
            payload,
        )
        .execute(self.pool())
        .await?
        .rows_affected()
            > 0)
    }

    /// Lists every non-terminal remote job with its owner state. The recovery
    /// pass never polls a remote: it only fences rows whose owner is gone and
    /// adopts pre-P3 parked items back into the claimable queue.
    pub async fn list_active_docling_remote_jobs(&self) -> Result<Vec<DoclingRecoveryRow>> {
        Ok(sqlx::query_file_as!(
            DoclingRecoveryRow,
            "src/sql/db/tasks/docling_remote_jobs/list_active_for_recovery.sql",
        )
        .fetch_all(self.pool())
        .await?)
    }

    /// Terminal owners enqueue cleanup and fence the exact remote job in one
    /// statement. Live queued/waiting owners retain their adoption mapping.
    pub async fn reconcile_docling_remote_state(&self) -> Result<DoclingRecoverySummary> {
        let mut summary = DoclingRecoverySummary::default();
        for row in self.list_active_docling_remote_jobs().await? {
            if !is_live_status(&row.task_status) || !is_live_status(&row.item_status) {
                let reason = if row.item_status == "failed" || row.task_status == "failed" {
                    "permanent_failure"
                } else {
                    "business_cancel"
                };
                summary.cancelled_remote_jobs += sqlx::query_file_scalar!(
                    "src/sql/db/tasks/docling_remote_jobs/enqueue_cleanup_for_item.sql",
                    row.id,
                    reason,
                )
                .fetch_all(self.pool())
                .await?
                .len() as u64;
                continue;
            }
            if is_docling_park(&row) {
                let Some(payload) = row.adopt_payload.as_ref() else {
                    continue;
                };
                if self
                    .requeue_docling_waiting_item(row.item_id, payload)
                    .await?
                {
                    self.recompute_task(row.task_id).await?;
                    summary.adopted_items += 1;
                }
                continue;
            }
        }
        Ok(summary)
    }
}
