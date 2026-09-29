use std::future::Future;
use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, Utc};
use scheduler::{
    ExecutionSlot, GuardedRunResult, GuardedRunner, ValkeyExecutionGuard, ValkeyLeaseConfig,
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

/// Compatibility entrypoint for the removed Docling poll sweep (issue 650
/// P3).
///
/// Docling submit/poll/fetch now blocks inside the task worker holding the
/// admitted parent slot, and crash/restart/cancel fencing for the durable
/// remote ids lives in the recovery pass (`maintenance.rs`). This function
/// stays with its signature so `main.rs` keeps compiling; it performs no
/// scheduling, claims nothing, and polls nothing.
pub async fn run_docling_sweep_scheduler(_app: Arc<Context69App>) -> Result<()> {
    info!("docling poll sweep is removed; docling conversions block inside the task worker");
    Ok(())
}
