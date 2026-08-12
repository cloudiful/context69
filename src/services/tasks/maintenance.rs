use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Days, Utc};
use context69_contracts::{
    CancelActiveTasksResponse, PurgeTasksResponse, TaskMaintenanceOverview,
    TaskMaintenanceSettings, TaskMaintenanceStats, TaskPurgeMode,
    UpdateTaskMaintenanceSettingsRequest,
};
use tokio::time::{MissedTickBehavior, interval};

use super::TaskService;
use crate::domain::UserRecord;

pub const CLEANUP_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
pub const CLEANUP_BATCH_SIZE: i64 = 1000;
pub const STAGED_OBJECT_GRACE_HOURS: i64 = 24;
pub const MIN_RETENTION_DAYS: i64 = 1;
pub const MAX_RETENTION_DAYS: i64 = 3650;

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
