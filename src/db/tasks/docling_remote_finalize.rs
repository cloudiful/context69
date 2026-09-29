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
    /// Atomically commits one inline success: marks the remote job
    /// `succeeded` and persists the fetched sections into its running item's
    /// payload in a single statement. Returns `Some(item_id)` on commit and
    /// `None` when either fence fails (row already terminal, or the item
    /// moved/lost its lease) with no partial write.
    pub async fn finish_docling_remote_job_with_sections(
        &self,
        job_id: Uuid,
        item_id: Uuid,
        item_lease: Uuid,
        sections_payload: &Value,
        remote_status: &str,
    ) -> Result<Option<Uuid>> {
        Ok(sqlx::query_file_scalar!(
            "src/sql/db/tasks/docling_remote_jobs/finish_with_sections.sql",
            job_id,
            item_id,
            sections_payload,
            remote_status,
            item_lease,
        )
        .fetch_optional(self.pool())
        .await?)
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

    /// Fences orphaned remote rows and adopts legacy parked items (issue 650
    /// P3 recovery). Per active row:
    ///
    /// - task or item terminal: cancel the row; the owner is done and no
    ///   worker will ever observe it again.
    /// - legacy `waiting/docling` park: requeue the item with unchanged
    ///   content, recompute the parent, then cancel the orphaned row so the
    ///   next claim adopts the item and the worker submits a fresh
    ///   conversion. Requeue runs before the cancel so a crash between the
    ///   two stays visible to the next pass (queued item plus active row is
    ///   cancelled below; a parked item keeps its row for re-adoption).
    /// - `running` item: never touched, whether its lease is live (an
    ///   in-flight blocking worker owns it) or expired (the claim reclaims
    ///   it with the active row intact so the resumed worker adopts the
    ///   tracked remote id instead of resubmitting).
    /// - `waiting` on another reason with an active row (a worker parked
    ///   after its inline budget with the conversion still referenced), or
    ///   `queued` with an active row (adoption transient): cancel the row so
    ///   the item becomes claimable and resumes with a fresh submit.
    pub async fn reconcile_docling_remote_state(&self) -> Result<DoclingRecoverySummary> {
        let mut summary = DoclingRecoverySummary::default();
        for row in self.list_active_docling_remote_jobs().await? {
            if !is_live_status(&row.task_status) || !is_live_status(&row.item_status) {
                if self
                    .cancel_active_docling_remote_job_for_item(
                        row.item_id,
                        Some("owning task/item is terminal"),
                    )
                    .await?
                    .is_some()
                {
                    summary.cancelled_remote_jobs += 1;
                }
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
                    if self
                        .cancel_active_docling_remote_job_for_item(
                            row.item_id,
                            Some("legacy parked item adopted for fresh submit"),
                        )
                        .await?
                        .is_some()
                    {
                        summary.cancelled_remote_jobs += 1;
                    }
                }
                continue;
            }
            if row.item_status == "running" {
                continue;
            }
            if self
                .cancel_active_docling_remote_job_for_item(
                    row.item_id,
                    Some("remote reference without a live owner"),
                )
                .await?
                .is_some()
            {
                summary.cancelled_remote_jobs += 1;
            }
        }
        Ok(summary)
    }
}
