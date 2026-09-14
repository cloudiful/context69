use std::{error::Error, fmt, time::Duration};

use anyhow::{Context, Result};

use crate::domain_errors::DomainError;
use chrono::Utc;
use context69_contracts::{
    CancelActiveTasksResponse, QuarantineStaleSubmittingRequest, QuarantineStaleSubmittingResponse,
    QuarantinedExternalJob, QueueDoclingRecoveryRequest, QueueDoclingRecoveryResponse,
    QueuedDoclingTask, RecoverDoclingTaskRequest, RecoverDoclingTaskResponse, RecoveredDoclingTask,
};
use tokio::time::{MissedTickBehavior, interval};
use uuid::Uuid;

use super::TaskService;
use crate::{domain::UserRecord, library_store::RecoveryAudit};
/// Default quarantine grace: only `submitting` rows older than this are
/// eligible. Well past the 10-minute admission reservation window so no
/// in-flight POST can still be completing.
pub const QUARANTINE_DEFAULT_GRACE_MINUTES: i64 = 30;
pub const QUARANTINE_MIN_GRACE_MINUTES: i64 = 10;
pub const QUARANTINE_MAX_GRACE_MINUTES: i64 = 10080;
pub const QUARANTINE_DEFAULT_LIMIT: i64 = 100;
pub const QUARANTINE_MAX_LIMIT: i64 = 1000;

#[derive(Debug)]
pub(crate) enum TaskMaintenanceError {
    BadRequest(String),
    Conflict(String),
    NotFound(String),
}

impl fmt::Display for TaskMaintenanceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadRequest(message) | Self::Conflict(message) | Self::NotFound(message) => {
                formatter.write_str(message)
            }
        }
    }
}

impl Error for TaskMaintenanceError {}

pub(super) fn start(service: &TaskService) {
    start_with_shutdown(service, tokio_util::sync::CancellationToken::new());
}

/// Wakeable source-cleanup wiring (issues 389/391): task history is never
/// auto-deleted, so only the shared source-cleanup dispatcher runs here
/// (startup drain, immediate wake after commit, 5-minute fallback).
/// `shutdown` cancels the dispatcher loop for tests and graceful shutdown;
/// production also aborts the task on process exit.
pub(super) fn start_with_shutdown(
    service: &TaskService,
    shutdown: tokio_util::sync::CancellationToken,
) {
    let release_service = service.clone();
    tokio::spawn(async move {
        let dispatcher = release_service
            .library()
            .source_cleanup_dispatcher()
            .unwrap_or_else(crate::services::library::SourceCleanupDispatcher::new);
        dispatcher
            .run(release_service.library().clone(), shutdown)
            .await;
    });
}

pub(crate) fn quarantine_grace_minutes(requested: Option<i64>) -> Result<i64> {
    let minutes = requested.unwrap_or(QUARANTINE_DEFAULT_GRACE_MINUTES);
    if !(QUARANTINE_MIN_GRACE_MINUTES..=QUARANTINE_MAX_GRACE_MINUTES).contains(&minutes) {
        return Err(TaskMaintenanceError::BadRequest(format!(
            "grace_minutes must be between {QUARANTINE_MIN_GRACE_MINUTES} and {QUARANTINE_MAX_GRACE_MINUTES}"
        ))
        .into());
    }
    Ok(minutes)
}

pub(crate) fn quarantine_limit(requested: Option<i64>) -> i64 {
    requested
        .unwrap_or(QUARANTINE_DEFAULT_LIMIT)
        .clamp(1, QUARANTINE_MAX_LIMIT)
}

pub(crate) fn require_non_empty_reason(reason: &str, what: &str) -> Result<String> {
    let trimmed = reason.trim();
    if trimmed.is_empty() {
        return Err(
            TaskMaintenanceError::BadRequest(format!("{what} reason must not be empty")).into(),
        );
    }
    Ok(trimmed.to_string())
}

impl TaskService {
    pub async fn admin_cancel_active_tasks(
        &self,
        actor: &UserRecord,
    ) -> Result<CancelActiveTasksResponse> {
        crate::services::auth::require_admin(actor)?;
        Ok(CancelActiveTasksResponse {
            cancelled_tasks: self.db().cancel_all_active_tasks().await?,
        })
    }

    /// Recover a Docling task whose external job is no longer polling usefully.
    /// The item is claimed with a real lease before any network request. It is
    /// parked on `external_job` after the fresh submission so the dispatcher
    /// cannot submit the same file a second time.
    pub async fn admin_recover_docling_task(
        &self,
        actor: &UserRecord,
        task_id: Uuid,
        request: &RecoverDoclingTaskRequest,
    ) -> Result<RecoverDoclingTaskResponse> {
        crate::services::auth::require_admin(actor)?;
        let reason = request.reason.trim();
        if reason.is_empty() {
            return Err(TaskMaintenanceError::BadRequest(
                "recovery reason must not be empty".to_string(),
            )
            .into());
        }
        let lease_token = Uuid::new_v4();
        let recovery = self.db().recover_docling_item(task_id, lease_token).await?;
        match recovery.reason.as_deref().unwrap_or("no_docling_item") {
            "ok" => {}
            "task_not_found" => {
                return Err(TaskMaintenanceError::NotFound("task not found".to_string()).into());
            }
            "task_terminal" => {
                return Err(
                    TaskMaintenanceError::Conflict("task is already terminal".to_string()).into(),
                );
            }
            "lease_active" => {
                return Err(TaskMaintenanceError::Conflict(
                    "task item has an active lease; wait for the worker to release it".to_string(),
                )
                .into());
            }
            "item_terminal" => {
                return Err(TaskMaintenanceError::Conflict(
                    "task item is already terminal".to_string(),
                )
                .into());
            }
            "active_external_job" => {
                return Err(TaskMaintenanceError::Conflict(
                    "task already has an active Docling external job".to_string(),
                )
                .into());
            }
            "already_recovered" => {
                let item_id = recovery.item_id.context("recovery returned no item id")?;
                let file_id = recovery.file_id;
                let remote_task_id = recovery
                    .remote_task_id
                    .context("recovery returned no active remote task id")?;
                return Ok(RecoverDoclingTaskResponse {
                    recovered: RecoveredDoclingTask {
                        task_id,
                        item_id,
                        old_remote_task_id: None,
                        old_remote_status: None,
                        new_remote_task_id: remote_task_id,
                        new_stage: "docling_poll".to_string(),
                        file_id,
                        recovered_at: Utc::now(),
                    },
                });
            }
            "dependency_waiting" => {
                return Err(TaskMaintenanceError::Conflict(
                    "task is already waiting for the Docling dependency probe".to_string(),
                )
                .into());
            }
            "uncertain_submission" => {
                return Err(TaskMaintenanceError::Conflict(
                    "Docling submission outcome is uncertain; quarantine the stale submitting job before recovery"
                        .to_string(),
                )
                .into());
            }
            "no_docling_item" => {
                return Err(
                    TaskMaintenanceError::Conflict("task has no Docling item".to_string()).into(),
                );
            }
            other => {
                return Err(
                    TaskMaintenanceError::Conflict(format!("recovery rejected: {other}")).into(),
                );
            }
        }
        let item_id = recovery.item_id.context("recovery returned no item id")?;
        let file_id = recovery
            .file_id
            .context("Docling recovery requires the item to carry a file_id")?;
        let attempt_id = recovery
            .attempt_id
            .context("recovery returned no attempt id")?;
        let _lease_guard = RecoveryLeaseGuard::start(self.db().clone(), item_id, lease_token);

        if let Some(next_attempt_at) = self
            .library()
            .dependency_wait_until("docling", lease_token)
            .await?
        {
            let waiting = self
                .db()
                .release_recovery_wait(item_id, lease_token, attempt_id, next_attempt_at)
                .await?;
            if !waiting {
                return Err(TaskMaintenanceError::Conflict(
                    "recovery lease was lost while waiting for the Docling gate".to_string(),
                )
                .into());
            }
            self.db().recompute_task(task_id).await?;
            return Err(TaskMaintenanceError::Conflict(
                "Docling dependency gate is not ready; retry recovery after the next probe"
                    .to_string(),
            )
            .into());
        }

        let existing = match self
            .library()
            .store()
            .supersede_external_job(
                item_id,
                crate::services::library::DOCLING_EXTERNAL_JOB_PROVIDER,
                reason,
            )
            .await
        {
            Ok(existing) => existing,
            Err(error) => {
                release_recovery_lease(self, task_id, item_id, lease_token).await;
                return Err(error);
            }
        };

        let submitted = match self
            .library()
            .submit_docling_job_for_task(item_id, file_id, lease_token, task_id)
            .await
        {
            Ok(submitted) => submitted,
            Err(error) => {
                let failure = self
                    .library()
                    .handle_task_ingest_failure(file_id, lease_token, error)
                    .await;
                if failure.retryable {
                    let updated = self
                        .db()
                        .wait_task_item(
                            task_id,
                            item_id,
                            lease_token,
                            "dependency",
                            failure.dependency_key.as_deref(),
                            Utc::now() + chrono::Duration::seconds(60),
                            Some(&failure.message),
                        )
                        .await?;
                    if !updated {
                        return Err(TaskMaintenanceError::Conflict(
                            "recovery lease was lost while recording submission failure"
                                .to_string(),
                        )
                        .into());
                    }
                } else {
                    let updated = self
                        .db()
                        .finish_task_item(
                            task_id,
                            item_id,
                            "failed",
                            None,
                            Some(&failure.stage),
                            Some(&failure.message),
                            false,
                            lease_token,
                            attempt_id,
                        )
                        .await?;
                    if !updated {
                        return Err(TaskMaintenanceError::Conflict(
                            "recovery lease was lost while recording submission failure"
                                .to_string(),
                        )
                        .into());
                    }
                }
                self.db().recompute_task(task_id).await?;
                return Err(DomainError::upstream_error(format!(
                    "fresh docling submission failed: {}",
                    failure.message
                ))
                .into());
            }
        };

        let stage_set = match self
            .db()
            .set_task_item_stage(task_id, item_id, lease_token, "docling_poll")
            .await
        {
            Ok(stage_set) => stage_set,
            Err(error) => {
                release_recovery_lease(self, task_id, item_id, lease_token).await;
                return Err(error);
            }
        };
        if !stage_set {
            return Err(TaskMaintenanceError::Conflict(
                "recovery lease was lost after Docling submission".to_string(),
            )
            .into());
        }
        let parked = self
            .db()
            .wait_task_item(
                task_id,
                item_id,
                lease_token,
                "external_job",
                None,
                submitted.next_poll_at,
                Some(&format!(
                    "docling task {} submitted; awaiting completion",
                    submitted.remote_task_id
                )),
            )
            .await?;
        if !parked {
            release_recovery_lease(self, task_id, item_id, lease_token).await;
            return Err(TaskMaintenanceError::Conflict(
                "recovery lease was lost while parking the external job".to_string(),
            )
            .into());
        }
        self.db().recompute_task(task_id).await?;
        if let Err(error) = self
            .library()
            .store()
            .record_recovery_audit(&RecoveryAudit {
                task_id,
                item_id,
                actor_user_id: actor.id,
                actor_login_name: &actor.login_name,
                reason,
                old_external_job_id: existing.old_external_job_id,
                old_remote_task_id: existing.old_remote_task_id.as_deref(),
                old_remote_status: existing.old_remote_status.as_deref(),
                old_status: existing.old_status.as_deref(),
                old_submission_count: existing.prior_submission_count,
                new_external_job_id: submitted.external_job_id,
                new_remote_task_id: &submitted.remote_task_id,
                new_submission_count: submitted.submission_count,
            })
            .await
        {
            return Err(DomainError::internal(format!(
                "Docling recovery completed but audit insertion failed: {error}"
            ))
            .into());
        }
        tracing::info!(
            task_id = %task_id,
            item_id = %item_id,
            old_status = existing.old_status.as_deref().unwrap_or("none"),
            old_submission_count = existing.prior_submission_count,
            new_submission_count = submitted.submission_count,
            "Docling recovery submitted a fresh external job"
        );
        self.notify_dispatch();
        Ok(RecoverDoclingTaskResponse {
            recovered: RecoveredDoclingTask {
                task_id,
                item_id,
                old_remote_task_id: existing.old_remote_task_id,
                old_remote_status: existing.old_remote_status,
                new_remote_task_id: submitted.remote_task_id,
                new_stage: "docling_poll".to_string(),
                file_id: Some(file_id),
                recovered_at: Utc::now(),
            },
        })
    }

    /// Requeue a recoverable Docling item without touching the network.
    ///
    /// Queue-only recovery (issue #118 phase 4): the item is persisted back
    /// to the `docling` scheduling queue and the dispatcher submits it later
    /// under the persistent admission ceiling, so bulk recovery cannot flood
    /// Docling with one POST per task. No attempt row and no remote job are
    /// created; a repeat call returns the already-queued item unchanged.
    /// The immediate POST path stays available via
    /// [`TaskService::admin_recover_docling_task`].
    pub async fn admin_queue_docling_recovery(
        &self,
        actor: &UserRecord,
        task_id: Uuid,
        request: &QueueDoclingRecoveryRequest,
    ) -> Result<QueueDoclingRecoveryResponse> {
        crate::services::auth::require_admin(actor)?;
        let reason = require_non_empty_reason(&request.reason, "recovery")?;
        let queued = self.db().queue_docling_recovery(task_id).await?;
        match queued.reason.as_deref().unwrap_or("no_docling_item") {
            "ok" => {}
            "already_queued" => {
                let item_id = queued.item_id.context("recovery returned no item id")?;
                return Ok(QueueDoclingRecoveryResponse {
                    queued: QueuedDoclingTask {
                        task_id,
                        item_id,
                        stage: "docling".to_string(),
                        file_id: queued.file_id,
                        queued_at: Utc::now(),
                        already_queued: true,
                    },
                });
            }
            "task_not_found" => {
                return Err(TaskMaintenanceError::NotFound("task not found".to_string()).into());
            }
            "task_terminal" => {
                return Err(
                    TaskMaintenanceError::Conflict("task is already terminal".to_string()).into(),
                );
            }
            "lease_active" => {
                return Err(TaskMaintenanceError::Conflict(
                    "task item has an active lease; wait for the worker to release it".to_string(),
                )
                .into());
            }
            "item_terminal" => {
                return Err(TaskMaintenanceError::Conflict(
                    "task item is already terminal".to_string(),
                )
                .into());
            }
            "active_external_job" => {
                return Err(TaskMaintenanceError::Conflict(
                    "task already has an active Docling external job".to_string(),
                )
                .into());
            }
            "uncertain_submission" => {
                return Err(TaskMaintenanceError::Conflict(
                    "Docling submission outcome is uncertain; quarantine the stale submitting job before recovery"
                        .to_string(),
                )
                .into());
            }
            "missing_file" => {
                return Err(TaskMaintenanceError::Conflict(
                    "Docling recovery requires the item to carry a file_id".to_string(),
                )
                .into());
            }
            "dependency_waiting" => {
                return Err(TaskMaintenanceError::Conflict(
                    "task is already waiting for the Docling dependency probe".to_string(),
                )
                .into());
            }
            "no_docling_item" => {
                return Err(
                    TaskMaintenanceError::Conflict("task has no Docling item".to_string()).into(),
                );
            }
            other => {
                return Err(
                    TaskMaintenanceError::Conflict(format!("recovery rejected: {other}")).into(),
                );
            }
        }
        let item_id = queued.item_id.context("recovery returned no item id")?;
        tracing::info!(
            task_id = %task_id,
            item_id = %item_id,
            reason = %reason,
            actor = %actor.login_name,
            "Docling queue-only recovery parked the item for dispatcher pickup",
        );
        self.notify_dispatch();
        Ok(QueueDoclingRecoveryResponse {
            queued: QueuedDoclingTask {
                task_id,
                item_id,
                stage: "docling".to_string(),
                file_id: queued.file_id,
                queued_at: Utc::now(),
                already_queued: false,
            },
        })
    }

    /// Isolate stale uncertain `submitting` Docling rows as `orphaned`.
    ///
    /// Only placeholder remote ids older than the grace cutoff on terminal
    /// parents are moved; live jobs, fresh rows, real remote ids, and
    /// non-terminal parents are left untouched and reported as skipped
    /// counts. The transition never claims the remote job was cancelled, and
    /// quarantined rows stop blocking terminal-task cleanup/purge. No
    /// background job calls this automatically.
    ///
    /// When `request.dry_run` is true, only eligibility stats are read and no
    /// row or audit row is written: the response carries `dry_run=true`, an
    /// empty `quarantined` list, `quarantined_count=0`, and the full
    /// `quarantinable_count` preview plus skipped counts. Admin authorization
    /// and `reason`/`grace_minutes`/`limit` validation still apply.
    pub async fn admin_quarantine_stale_submitting(
        &self,
        actor: &UserRecord,
        request: &QuarantineStaleSubmittingRequest,
    ) -> Result<QuarantineStaleSubmittingResponse> {
        crate::services::auth::require_admin(actor)?;
        let reason = require_non_empty_reason(&request.reason, "quarantine")?;
        let grace_minutes = quarantine_grace_minutes(request.grace_minutes)?;
        let limit = quarantine_limit(request.limit);
        let dry_run = request.dry_run.unwrap_or(false);
        let cutoff = Utc::now() - chrono::Duration::minutes(grace_minutes);
        let pattern = crate::library_store::SUBMITTING_PLACEHOLDER_PATTERN;
        if dry_run {
            let stats = self
                .library()
                .store()
                .quarantine_submitting_stats(cutoff, pattern)
                .await?;
            tracing::info!(
                dry_run = true,
                quarantinable = stats.quarantinable_count,
                uncertain_total = stats.uncertain_submitting_count,
                orphaned_total = stats.orphaned_count,
                skipped_non_terminal = stats.skipped_non_terminal_count,
                skipped_fresh = stats.skipped_fresh_count,
                skipped_real_remote = stats.skipped_real_remote_count,
                grace_minutes = grace_minutes,
                limit = limit,
                actor = %actor.login_name,
                reason = %reason,
                "stale Docling submitting quarantine dry-run preview",
            );
            return Ok(QuarantineStaleSubmittingResponse {
                quarantined: Vec::new(),
                quarantined_count: 0,
                skipped_non_terminal: stats.skipped_non_terminal_count,
                skipped_fresh: stats.skipped_fresh_count,
                skipped_real_remote: stats.skipped_real_remote_count,
                dry_run: true,
                quarantinable_count: stats.quarantinable_count,
            });
        }
        let quarantined = self
            .library()
            .store()
            .quarantine_stale_submitting(
                &reason,
                &actor.login_name,
                actor.id,
                cutoff,
                pattern,
                limit,
            )
            .await?;
        let stats = self
            .library()
            .store()
            .quarantine_submitting_stats(cutoff, pattern)
            .await?;
        tracing::info!(
            dry_run = false,
            quarantined = quarantined.len(),
            old_status_sample = quarantined
                .first()
                .and_then(|row| row.old_status.as_deref())
                .unwrap_or("none"),
            uncertain_total = stats.uncertain_submitting_count,
            quarantinable = stats.quarantinable_count,
            orphaned_total = stats.orphaned_count,
            skipped_non_terminal = stats.skipped_non_terminal_count,
            skipped_fresh = stats.skipped_fresh_count,
            skipped_real_remote = stats.skipped_real_remote_count,
            grace_minutes = grace_minutes,
            actor = %actor.login_name,
            reason = %reason,
            "stale Docling submitting quarantine completed",
        );
        Ok(QuarantineStaleSubmittingResponse {
            quarantined: quarantined
                .iter()
                .map(|row| QuarantinedExternalJob {
                    external_job_id: row.external_job_id,
                    task_id: row.task_id,
                    item_id: row.item_id,
                    old_remote_task_id: row.old_remote_task_id.clone(),
                    quarantined_at: row.quarantined_at.unwrap_or_else(Utc::now),
                })
                .collect::<Vec<_>>(),
            quarantined_count: quarantined.len() as i64,
            skipped_non_terminal: stats.skipped_non_terminal_count,
            skipped_fresh: stats.skipped_fresh_count,
            skipped_real_remote: stats.skipped_real_remote_count,
            dry_run: false,
            quarantinable_count: stats.quarantinable_count,
        })
    }
}

async fn release_recovery_lease(
    service: &TaskService,
    task_id: Uuid,
    item_id: Uuid,
    lease_token: Uuid,
) {
    if let Err(error) = service
        .db()
        .wait_task_item(
            task_id,
            item_id,
            lease_token,
            "backoff",
            None,
            Utc::now() + chrono::Duration::seconds(60),
            Some("Docling recovery could not complete; retrying later"),
        )
        .await
    {
        tracing::error!(
            %error,
            task_id = %task_id,
            item_id = %item_id,
            "failed to release Docling recovery lease"
        );
    }
}

struct RecoveryLeaseGuard(tokio::task::JoinHandle<()>);

impl RecoveryLeaseGuard {
    fn start(db: crate::db::Database, item_id: Uuid, lease_token: Uuid) -> Self {
        Self(tokio::spawn(async move {
            let mut ticks = interval(Duration::from_secs(30));
            ticks.set_missed_tick_behavior(MissedTickBehavior::Skip);
            loop {
                ticks.tick().await;
                match db.heartbeat_task_item(item_id, lease_token).await {
                    Ok(true) => {}
                    Ok(false) | Err(_) => break,
                }
            }
        }))
    }
}

impl Drop for RecoveryLeaseGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::{
        QUARANTINE_DEFAULT_GRACE_MINUTES, QUARANTINE_DEFAULT_LIMIT, QUARANTINE_MAX_GRACE_MINUTES,
        QUARANTINE_MAX_LIMIT, QUARANTINE_MIN_GRACE_MINUTES, quarantine_grace_minutes,
        quarantine_limit, require_non_empty_reason,
    };

    #[test]
    fn quarantine_grace_defaults_and_bounds() {
        assert_eq!(
            quarantine_grace_minutes(None).expect("default grace"),
            QUARANTINE_DEFAULT_GRACE_MINUTES
        );
        assert_eq!(
            quarantine_grace_minutes(Some(60)).expect("custom grace"),
            60
        );
        for invalid in [
            0,
            QUARANTINE_MIN_GRACE_MINUTES - 1,
            QUARANTINE_MAX_GRACE_MINUTES + 1,
            i64::MAX,
        ] {
            let error = quarantine_grace_minutes(Some(invalid)).expect_err("invalid grace");
            assert!(error.to_string().contains("grace_minutes"));
        }
    }

    #[test]
    fn quarantine_limit_defaults_and_clamps() {
        assert_eq!(quarantine_limit(None), QUARANTINE_DEFAULT_LIMIT);
        assert_eq!(quarantine_limit(Some(10)), 10);
        assert_eq!(quarantine_limit(Some(0)), 1);
        assert_eq!(quarantine_limit(Some(-5)), 1);
        assert_eq!(quarantine_limit(Some(i64::MAX)), QUARANTINE_MAX_LIMIT);
    }

    #[test]
    fn empty_reasons_are_rejected() {
        for empty in ["", "   ", "\n\t "] {
            let error = require_non_empty_reason(empty, "recovery").expect_err("empty reason");
            assert!(error.to_string().contains("must not be empty"));
        }
        assert_eq!(
            require_non_empty_reason("  bulk requeue  ", "recovery").expect("trimmed"),
            "bulk requeue"
        );
    }
}
