use std::future::Future;
use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, Utc};
use scheduler::{
    CoordinatedLeaseConfig, ExecutionSlot, GuardedRunResult, GuardedRunner, InMemoryStateStore,
    Job, OverlapPolicy, Schedule, Scheduler, SchedulerConfig, Task, TaskContext,
    ValkeyCoordinatedStateStore, ValkeyExecutionGuard, ValkeyLeaseConfig,
};
use tracing::info;

use crate::services::app::Context69App;

pub const SCHEDULER_VALKEY_KEY_PREFIX: &str = "context69:scheduler:job-state:";
pub const SCHEDULER_EXECUTION_LEASE_PREFIX: &str = "context69:scheduler:execution-lease:";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManualRunResult<T> {
    Completed(T),
    Contended,
}

pub async fn run_manual_sync_guarded<F, Fut, T>(
    app: Arc<Context69App>,
    resource_id: impl Into<String>,
    run: F,
) -> Result<ManualRunResult<T>>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let Some(valkey_url) = app.config.scheduler.valkey_url.as_deref() else {
        return run().await.map(ManualRunResult::Completed);
    };

    let guard = build_valkey_execution_guard(
        valkey_url,
        ValkeyLeaseConfig {
            ttl: app.config.scheduler.execution_guard_ttl,
            renew_interval: app.config.scheduler.execution_guard_renew_interval,
        },
    )
    .await?;
    let runner = GuardedRunner::new(guard);
    let guarded = runner
        .run(
            ExecutionSlot::for_resource(app.config.scheduler.job_id.clone(), resource_id.into()),
            run,
        )
        .await?;

    Ok(match guarded {
        GuardedRunResult::Completed(result) => ManualRunResult::Completed(result?),
        GuardedRunResult::Contended => ManualRunResult::Contended,
    })
}

pub async fn build_valkey_execution_guard(
    valkey_url: &str,
    lease_config: ValkeyLeaseConfig,
) -> Result<ValkeyExecutionGuard> {
    ValkeyExecutionGuard::with_prefix(valkey_url, SCHEDULER_EXECUTION_LEASE_PREFIX, lease_config)
        .await
        .map_err(Into::into)
}

pub fn startup_execution_slot_at() -> DateTime<Utc> {
    DateTime::<Utc>::UNIX_EPOCH
}

/// Dedicated job id for the durable Docling poll sweep. Derived from the
/// configured sync job id so coordinated Valkey state never collides with
/// `sync_all` while sharing the same prefixes and lease config.
pub fn docling_sweep_job_id(app: &Context69App) -> String {
    format!("{}-docling-poll", app.config.scheduler.job_id)
}

/// Interval for the single docling-poll-sweep job. Fixed at 2s with
/// `OverlapPolicy::Forbid` per the chosen architecture: due rows carry their
/// own `next_poll_at` backoff, so the tick is only a wake-up, never the poll
/// cadence itself.
pub fn docling_sweep_interval() -> std::time::Duration {
    std::time::Duration::from_secs(2)
}

pub async fn run_docling_sweep_scheduler(app: Arc<Context69App>) -> Result<()> {
    let valkey_url = app.config.scheduler.valkey_url.clone();
    let execution_guard_ttl = app.config.scheduler.execution_guard_ttl;
    let execution_guard_renew_interval = app.config.scheduler.execution_guard_renew_interval;

    let job = Job::new(
        docling_sweep_job_id(&app),
        Schedule::Interval(docling_sweep_interval()),
        app,
        Task::from_async(async |context: TaskContext<Arc<Context69App>>| {
            match crate::services::tasks::docling_sweep::run_docling_poll_sweep_once(
                &context.deps.db,
                &context.deps.library,
            )
            .await
            {
                Ok(summary) => {
                    if crate::services::tasks::docling_sweep::sweep_moved_items(&summary) {
                        context.deps.tasks.notify_dispatch();
                    }
                    if summary.errors > 0 {
                        Err(format!("docling sweep had {} errors", summary.errors))
                    } else {
                        Ok(())
                    }
                }
                Err(error) => Err(error.to_string()),
            }
        }),
    )
    .with_overlap_policy(OverlapPolicy::Forbid);

    let report = if let Some(valkey_url) = valkey_url.as_deref() {
        let store = ValkeyCoordinatedStateStore::with_prefixes(
            valkey_url,
            SCHEDULER_VALKEY_KEY_PREFIX,
            SCHEDULER_EXECUTION_LEASE_PREFIX,
        )
        .await?;
        Scheduler::with_coordinated_state_store(
            SchedulerConfig::default(),
            store,
            CoordinatedLeaseConfig {
                ttl: execution_guard_ttl,
                renew_interval: execution_guard_renew_interval,
            },
        )
        .run(job)
        .await?
    } else {
        Scheduler::new(SchedulerConfig::default(), InMemoryStateStore::new())
            .run(job)
            .await?
    };
    info!(
        runs = report.history.len(),
        next_run_at = ?report.state.next_run_at,
        "docling sweep scheduler loop exited"
    );
    Ok(())
}
