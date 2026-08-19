use std::{error::Error, fmt, time::Duration};

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Days, Utc};
use context69_contracts::{
    CancelActiveTasksResponse, PurgeTasksResponse, RecoverDoclingTaskRequest,
    RecoverDoclingTaskResponse, RecoveredDoclingTask, TaskMaintenanceOverview,
    TaskMaintenanceSettings, TaskMaintenanceStats, TaskPurgeMode,
    UpdateTaskMaintenanceSettingsRequest,
};
use tokio::time::{MissedTickBehavior, interval};
use uuid::Uuid;

use super::TaskService;
use crate::{domain::UserRecord, library_store::RecoveryAudit};

pub const CLEANUP_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
pub const CLEANUP_BATCH_SIZE: i64 = 1000;
pub const STAGED_OBJECT_GRACE_HOURS: i64 = 24;
pub const MIN_RETENTION_DAYS: i64 = 1;
pub const MAX_RETENTION_DAYS: i64 = 3650;

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
    let service = service.clone();
    tokio::spawn(async move {
        let mut cycle = interval(CLEANUP_INTERVAL);
        cycle.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            if let Err(error) = run_cleanup(&service).await {
                tracing::warn!(%error, "task history cleanup cycle failed; retrying next cycle");
            }
            cycle.tick().await;
        }
    });
}

async fn run_cleanup(service: &TaskService) -> Result<()> {
    let Some(settings) = service.db().get_task_maintenance_settings().await? else {
        return Err(anyhow!("task maintenance settings are missing"));
    };
    let mut deleted = 0i64;
    let cutoff = cutoff_at(Utc::now(), settings.retention_days);
    if settings.cleanup_enabled {
        loop {
            let batch = service
                .db()
                .cleanup_expired_terminal_tasks(cutoff, CLEANUP_BATCH_SIZE)
                .await?;
            let batch_len = batch.len() as i64;
            deleted += batch_len;
            if batch_len < CLEANUP_BATCH_SIZE {
                break;
            }
        }
    } else {
        tracing::info!("task history auto-cleanup is disabled");
    }
    let staged_cutoff = Utc::now() - chrono::Duration::hours(STAGED_OBJECT_GRACE_HOURS);
    let deleted_objects = service
        .library()
        .sweep_orphaned_storage_objects(staged_cutoff, CLEANUP_BATCH_SIZE)
        .await?;
    tracing::info!(
        deleted_tasks = deleted,
        deleted_staged_objects = deleted_objects,
        retention_days = settings.retention_days,
        cutoff = %cutoff,
        "task history cleanup cycle completed"
    );
    Ok(())
}

pub(crate) fn cutoff_at(now: DateTime<Utc>, retention_days: i64) -> DateTime<Utc> {
    let days = u64::try_from(retention_days.max(MIN_RETENTION_DAYS)).unwrap_or(u64::MAX);
    now.checked_sub_days(Days::new(days))
        .unwrap_or_else(|| now - chrono::Duration::days(1))
}

pub(crate) fn validate_retention_days(retention_days: i64) -> Result<()> {
    if !(MIN_RETENTION_DAYS..=MAX_RETENTION_DAYS).contains(&retention_days) {
        return Err(anyhow!(
            "retention_days must be between {} and {}",
            MIN_RETENTION_DAYS,
            MAX_RETENTION_DAYS
        ));
    }
    Ok(())
}

pub(crate) fn require_no_active_tasks(active_count: i64, mode: TaskPurgeMode) -> Result<()> {
    if mode == TaskPurgeMode::AllTerminal && active_count > 0 {
        return Err(anyhow!(
            "active tasks must be cancelled before purging the full task history"
        ));
    }
    Ok(())
}

impl TaskService {
    pub async fn admin_maintenance_overview(
        &self,
        actor: &UserRecord,
    ) -> Result<TaskMaintenanceOverview> {
        crate::services::auth::require_admin(actor)?;
        let settings = self.maintenance_settings().await?;
        let stats = self.maintenance_stats(&settings).await?;
        Ok(TaskMaintenanceOverview { settings, stats })
    }

    pub async fn admin_update_maintenance_settings(
        &self,
        actor: &UserRecord,
        request: &UpdateTaskMaintenanceSettingsRequest,
    ) -> Result<TaskMaintenanceOverview> {
        crate::services::auth::require_admin(actor)?;
        validate_retention_days(request.retention_days)?;
        let settings = self
            .db()
            .update_task_maintenance_settings(request.cleanup_enabled, request.retention_days)
            .await?
            .into();
        let stats = self.maintenance_stats(&settings).await?;
        Ok(TaskMaintenanceOverview { settings, stats })
    }

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
                return Err(anyhow!(
                    "fresh docling submission failed: {}",
                    failure.message
                ));
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
                old_submission_count: existing.prior_submission_count,
                new_external_job_id: submitted.external_job_id,
                new_remote_task_id: &submitted.remote_task_id,
                new_submission_count: submitted.submission_count,
            })
            .await
        {
            return Err(anyhow!(
                "Docling recovery completed but audit insertion failed: {error}"
            ));
        }
        tracing::info!(
            task_id = %task_id,
            item_id = %item_id,
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

    pub async fn admin_purge_tasks(
        &self,
        actor: &UserRecord,
        mode: TaskPurgeMode,
    ) -> Result<PurgeTasksResponse> {
        crate::services::auth::require_admin(actor)?;
        let settings = self.maintenance_settings().await?;
        let stats = self.maintenance_stats(&settings).await?;
        require_no_active_tasks(stats.active, mode)?;
        let cutoff = cutoff_at(Utc::now(), settings.retention_days);
        let mut deleted = 0i64;
        loop {
            let batch = match mode {
                TaskPurgeMode::Expired => {
                    self.db()
                        .cleanup_expired_terminal_tasks(cutoff, CLEANUP_BATCH_SIZE)
                        .await?
                }
                TaskPurgeMode::AllTerminal => {
                    self.db().purge_terminal_tasks(CLEANUP_BATCH_SIZE).await?
                }
            };
            let batch_len = batch.len() as i64;
            deleted += batch_len;
            if batch_len < CLEANUP_BATCH_SIZE {
                break;
            }
        }
        Ok(PurgeTasksResponse {
            deleted_tasks: deleted,
        })
    }

    pub async fn maintenance_settings(&self) -> Result<TaskMaintenanceSettings> {
        Ok(self
            .db()
            .get_task_maintenance_settings()
            .await?
            .context("task maintenance settings are missing")?
            .into())
    }

    async fn maintenance_stats(
        &self,
        settings: &TaskMaintenanceSettings,
    ) -> Result<TaskMaintenanceStats> {
        let cutoff = cutoff_at(Utc::now(), settings.retention_days);
        let stats = self.db().task_maintenance_stats(cutoff).await?;
        Ok(TaskMaintenanceStats {
            total: stats.total_count,
            queued: stats.queued_count,
            running: stats.running_count,
            waiting: stats.waiting_count,
            succeeded: stats.succeeded_count,
            failed: stats.failed_count,
            cancelled: stats.cancelled_count,
            active: stats.active_count,
            expired_terminal: stats.expired_terminal_count,
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

impl From<crate::db::StoredTaskMaintenanceSettings> for TaskMaintenanceSettings {
    fn from(settings: crate::db::StoredTaskMaintenanceSettings) -> Self {
        Self {
            cleanup_enabled: settings.cleanup_enabled,
            retention_days: settings.retention_days,
            updated_at: settings.updated_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::{
        MAX_RETENTION_DAYS, MIN_RETENTION_DAYS, cutoff_at, require_no_active_tasks,
        validate_retention_days,
    };
    use context69_contracts::TaskPurgeMode;

    #[test]
    fn retention_days_boundaries_are_accepted() {
        validate_retention_days(MIN_RETENTION_DAYS).expect("minimum accepted");
        validate_retention_days(30).expect("default accepted");
        validate_retention_days(MAX_RETENTION_DAYS).expect("maximum accepted");
    }

    #[test]
    fn retention_days_outside_bounds_are_rejected() {
        for invalid in [0, -1, MAX_RETENTION_DAYS + 1, i64::MAX] {
            let error = validate_retention_days(invalid).expect_err("invalid retention");
            assert!(error.to_string().contains("between"));
        }
    }

    #[test]
    fn cutoff_uses_full_retention_window() {
        let now = Utc::now();
        let cutoff = cutoff_at(now, 30);
        assert_eq!((now - cutoff).num_days(), 30);
    }

    #[test]
    fn all_terminal_purge_requires_zero_active_tasks() {
        require_no_active_tasks(0, TaskPurgeMode::AllTerminal).expect("empty queue purges");
        require_no_active_tasks(3, TaskPurgeMode::Expired)
            .expect("expired purge runs with active tasks");
        let error = require_no_active_tasks(1, TaskPurgeMode::AllTerminal)
            .expect_err("active tasks block all-terminal purge");
        assert!(error.to_string().contains("active tasks"));
    }
}
