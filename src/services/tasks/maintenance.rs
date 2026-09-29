use std::time::Duration;

use anyhow::Result;

use context69_contracts::CancelActiveTasksResponse;

use super::TaskService;
use crate::domain::UserRecord;

/// Interval for the Docling remote-reference recovery pass. The pass never
/// polls a remote: it only fences orphaned rows and adopts pre-P3 parked
/// items, so a 30s cadence (matching the dispatcher recovery tick) bounds
/// orphan lifetime without adding a poll queue.
const DOCLING_RECOVERY_INTERVAL: Duration = Duration::from_secs(30);

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
    let release_shutdown = shutdown.clone();
    tokio::spawn(async move {
        let dispatcher = release_service
            .library()
            .source_cleanup_dispatcher()
            .unwrap_or_default();
        dispatcher
            .run(release_service.library().clone(), release_shutdown)
            .await;
    });
    // Docling remote-reference recovery (issue 650 P3): one pass at startup
    // so rows left by the previous process are fenced before the first
    // dispatch drain, then the periodic pass. Shutdown-aware for tests.
    let recovery_service = service.clone();
    tokio::spawn(async move {
        run_docling_recovery(&recovery_service, shutdown).await;
    });
}

/// Recovery loop for durable Docling remote ids: fences rows whose
/// owner is gone (crash/restart/cancel) and adopts legacy parked items back
/// into the claimable queue. Never polls, fetches, or claims remote work;
/// the blocking worker owns the conversion lifecycle.
async fn run_docling_recovery(
    service: &TaskService,
    shutdown: tokio_util::sync::CancellationToken,
) {
    reconcile_once(service).await;
    let mut tick = tokio::time::interval(DOCLING_RECOVERY_INTERVAL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // The first interval tick completes immediately. Consume it so the first
    // periodic pass runs a full interval after the startup pass above.
    tick.tick().await;
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => break,
            _ = tick.tick() => reconcile_once(service).await,
        }
    }
}

async fn reconcile_once(service: &TaskService) {
    match service.db().reconcile_docling_remote_state().await {
        Ok(summary) => {
            if summary.adopted_items + summary.cancelled_remote_jobs > 0 {
                tracing::info!(
                    target: "docling_recovery",
                    adopted = summary.adopted_items,
                    cancelled = summary.cancelled_remote_jobs,
                    "docling remote recovery fenced rows",
                );
                if summary.adopted_items > 0 {
                    service.notify_dispatch();
                }
            }
        }
        Err(error) => {
            tracing::warn!(%error, "docling remote recovery failed; continuing");
        }
    }
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
}
