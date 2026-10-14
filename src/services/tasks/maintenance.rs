use std::time::Duration;

use anyhow::Result;

use context69_contracts::{
    CancelActiveTasksResponse, DoclingRemoteTasksQuery, DoclingRemoteTasksResponse,
};

use super::TaskService;
use crate::domain::UserRecord;

/// Cadence for local-owner recovery and bounded detached remote cleanup.
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
    if let Err(_error) = super::docling_cleanup::run_once(service, &shutdown).await {
        tracing::warn!("docling cleanup pass failed; pending claims will expire");
    }
    let mut tick = tokio::time::interval(DOCLING_RECOVERY_INTERVAL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // The first interval tick completes immediately. Consume it so the first
    // periodic pass runs a full interval after the startup pass above.
    tick.tick().await;
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => break,
            _ = tick.tick() => {
                reconcile_once(service).await;
                if let Err(_error) = super::docling_cleanup::run_once(service, &shutdown).await {
                    tracing::warn!("docling cleanup pass failed; pending claims will expire");
                }
            },
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
        Err(_error) => {
            tracing::warn!("docling remote recovery failed; continuing");
        }
    }
}

fn validate_remote_query(query: &DoclingRemoteTasksQuery) -> Result<()> {
    query
        .validate()
        .map_err(|error| crate::domain_errors::DomainError::invalid_argument(error.to_string()))?;
    Ok(())
}

fn remote_tasks_with_cleanup(
    remote: Result<DoclingRemoteTasksResponse>,
    cleanup: Vec<context69_contracts::DoclingCleanupTask>,
) -> DoclingRemoteTasksResponse {
    let mut response = remote.unwrap_or_else(|_| DoclingRemoteTasksResponse {
        items: Vec::new(),
        has_more: false,
        cleanup: Vec::new(),
        remote_error: Some(
            "Remote activity unavailable; cleanup history is still available".into(),
        ),
    });
    response.cleanup = cleanup;
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_remote_limits_are_http_400_before_io() {
        for limit in [0, 101] {
            let error =
                validate_remote_query(&DoclingRemoteTasksQuery { limit, offset: 0 }).unwrap_err();
            assert_eq!(
                crate::domain_errors::status_for_error(&error),
                axum::http::StatusCode::BAD_REQUEST
            );
        }
        assert!(
            validate_remote_query(&DoclingRemoteTasksQuery {
                limit: 100,
                offset: 0
            })
            .is_ok()
        );
    }

    #[test]
    fn upstream_errors_cannot_hide_cleanup_or_disclose_details() {
        let entry = context69_contracts::DoclingCleanupTask {
            id: uuid::Uuid::new_v4(),
            task_id: uuid::Uuid::new_v4(),
            remote_task_id: "owned".into(),
            status: "unsupported".into(),
            outcome: None,
            attempt_count: 1,
            next_attempt_at: chrono::Utc::now(),
            cleanup_deadline_at: chrono::Utc::now(),
        };
        let failed = remote_tasks_with_cleanup(
            Err(anyhow::anyhow!(
                "https://secret:password@private.invalid raw upstream body"
            )),
            vec![entry.clone()],
        );
        assert_eq!(failed.cleanup, vec![entry]);
        assert!(failed.items.is_empty());
        assert!(!failed.has_more);
        let error = failed.remote_error.unwrap();
        assert!(error.len() < 128);
        assert!(!error.contains("password"));
        assert!(!error.contains("private.invalid"));
        let healthy = remote_tasks_with_cleanup(
            Ok(DoclingRemoteTasksResponse {
                items: Vec::new(),
                has_more: true,
                cleanup: Vec::new(),
                remote_error: None,
            }),
            failed.cleanup,
        );
        assert!(healthy.has_more);
        assert!(healthy.remote_error.is_none());
        assert_eq!(healthy.cleanup.len(), 1);
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

    /// Bounded admin read of the fork's active Docling task identities.
    ///
    /// Issue 84 P1: uses the saved Docling settings only, validates the
    /// `limit`/`offset` bounds, and returns the fork's `has_more` verbatim.
    /// Non-admin callers are denied before any remote access.
    pub async fn admin_docling_remote_tasks(
        &self,
        actor: &UserRecord,
        query: &DoclingRemoteTasksQuery,
    ) -> Result<DoclingRemoteTasksResponse> {
        crate::services::auth::require_admin(actor)?;
        validate_remote_query(query)?;
        let cleanup = self
            .db()
            .list_recent_docling_cleanup(100)
            .await?
            .into_iter()
            .map(|row| context69_contracts::DoclingCleanupTask {
                id: row.id,
                task_id: row.task_id,
                remote_task_id: row.remote_task_id,
                status: row.status,
                outcome: row.outcome,
                attempt_count: row.attempt_count,
                next_attempt_at: row.next_attempt_at,
                cleanup_deadline_at: row.cleanup_deadline_at,
            })
            .collect();
        let remote = self
            .library()
            .list_docling_remote_tasks(query.limit, query.offset)
            .await;
        Ok(remote_tasks_with_cleanup(remote, cleanup))
    }

    pub async fn admin_docling_cleanup(
        &self,
        actor: &UserRecord,
    ) -> Result<context69_contracts::DoclingCleanupResponse> {
        crate::services::auth::require_admin(actor)?;
        let woken = self.db().wake_due_docling_cleanup().await?.len() as u32;
        Ok(context69_contracts::DoclingCleanupResponse { woken })
    }
}
