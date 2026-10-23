use std::time::Duration;

use anyhow::Result;

use context69_contracts::pagination::OffsetPageQuery;
use context69_contracts::{
    CancelActiveTasksResponse, DoclingCleanupPageResponse, DoclingCleanupQuery,
    DoclingRemoteTasksQuery, DoclingRemoteTasksResponse, Pagination,
};

use super::TaskService;
use crate::domain::UserRecord;

/// Cadence for local-owner recovery and bounded detached remote cleanup.
const DOCLING_RECOVERY_INTERVAL: Duration = Duration::from_secs(30);

pub(super) fn start(service: &TaskService) {
    start_with_shutdown(service, tokio_util::sync::CancellationToken::new());
}

/// Owned handles for the maintenance writers started with a recovery profile.
pub(crate) struct MaintenanceHandles {
    source_cleanup: Option<tokio::task::JoinHandle<()>>,
    docling_recovery: Option<tokio::task::JoinHandle<()>>,
}

impl MaintenanceHandles {
    /// Await both loops after their shared token has been cancelled, so no
    /// statement can still be running when the caller migrates.
    ///
    /// Cancellation-safe: each handle is awaited by mutable reference and only
    /// released after it is observed to finish, so an interrupted wait can be
    /// resumed by the bounded stop.
    pub(crate) async fn wait(&mut self) {
        if let Some(handle) = self.source_cleanup.as_mut() {
            let _ = (&mut *handle).await;
            self.source_cleanup = None;
        }
        if let Some(handle) = self.docling_recovery.as_mut() {
            let _ = (&mut *handle).await;
            self.docling_recovery = None;
        }
    }
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
    drop(start_with_shutdown_owned(service, shutdown));
}

/// Shutdown-aware start that returns both owned loop handles.
pub(super) fn start_with_shutdown_owned(
    service: &TaskService,
    shutdown: tokio_util::sync::CancellationToken,
) -> MaintenanceHandles {
    let release_service = service.clone();
    let release_shutdown = shutdown.clone();
    let source_cleanup = tokio::spawn(async move {
        // Fail-closed startup gate: no source-cleanup write runs while the
        // execution latch reports abandoned legacy work or an unusable library
        // schema. The latch logs the blocker once and never re-evaluates.
        if !release_service.execution_readiness().await.is_ready() {
            return;
        }
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
    let docling_recovery = tokio::spawn(async move {
        // Same fail-closed startup gate as the source-cleanup loop: no Docling
        // recovery, adoption or cleanup runs while the latch is blocked.
        if !recovery_service.execution_readiness().await.is_ready() {
            return;
        }
        run_docling_recovery(&recovery_service, shutdown).await;
    });
    MaintenanceHandles {
        source_cleanup: Some(source_cleanup),
        docling_recovery: Some(docling_recovery),
    }
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

fn validate_cleanup_query(query: &DoclingCleanupQuery) -> Result<()> {
    query
        .validate()
        .map_err(|error| crate::domain_errors::DomainError::invalid_argument(error.to_string()))?;
    Ok(())
}

/// Exact worker-written diagnostics that prove a cleanup cause.
///
/// Only these constants (from `super::docling_cleanup`) may map to a
/// specific reason; anything else is `Unspecified` so raw text never reaches
/// the UI. Comparisons are exact equality, never substring or URL parsing.
const MISSING_ENDPOINT_ERROR: &str = "Submit endpoint unavailable; remote termination unconfirmed";
const DEADLINE_ERROR: &str = "Cleanup deadline exceeded; remote termination unconfirmed";
const CANCEL_AUTH_ERROR: &str = "Remote cancellation requires unsupported authentication";
const OBSERVE_AUTH_ERROR: &str = "Remote observation requires unsupported authentication";
const NOT_FOUND_ERROR: &str = "Remote task not found; termination not confirmed";
const NOT_FOUND_OUTCOME: &str = "not_found";

/// Backend-owned cause for one cleanup row, derived only from the stored
/// `status`/`outcome` and the exact worker-written `last_error`.
///
/// Returns `None` for healthy rows (pending/cancelling/done with no issue),
/// a specific code for exact known diagnostics, and `Unspecified` for any
/// other stored error (including `unsupported` with missing detail). No extra
/// SQL or endpoint reads; `last_error` comes from the already-projected row.
fn docling_cleanup_reason_code(
    status: &str,
    outcome: Option<&str>,
    last_error: Option<&str>,
) -> Option<context69_contracts::DoclingCleanupReasonCode> {
    use context69_contracts::DoclingCleanupReasonCode as Code;
    match last_error {
        Some(error) if error == MISSING_ENDPOINT_ERROR => return Some(Code::MissingSubmitEndpoint),
        Some(error) if error == DEADLINE_ERROR => return Some(Code::DeadlineExceeded),
        Some(error) if error == CANCEL_AUTH_ERROR || error == OBSERVE_AUTH_ERROR => {
            return Some(Code::AuthenticationFailed);
        }
        Some(error) if error == NOT_FOUND_ERROR => return Some(Code::RemoteNotFound),
        _ => {}
    }
    if outcome == Some(NOT_FOUND_OUTCOME) {
        return Some(Code::RemoteNotFound);
    }
    if status == "awaiting_terminal" {
        return Some(Code::AwaitingRemoteTerminal);
    }
    if status == "unsupported" || status == "failed" {
        return Some(Code::Unspecified);
    }
    if last_error.is_some() {
        return Some(Code::Unspecified);
    }
    None
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

    #[tokio::test]
    async fn interrupted_maintenance_wait_keeps_each_owned_loop_joinable() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        use std::time::Duration;

        let source_done = Arc::new(AtomicBool::new(false));
        let docling_done = Arc::new(AtomicBool::new(false));
        let (release, released) = tokio::sync::oneshot::channel::<()>();
        let source_flag = source_done.clone();
        let docling_flag = docling_done.clone();
        let mut handles = MaintenanceHandles {
            source_cleanup: Some(tokio::spawn({
                let released = released;
                async move {
                    source_flag.store(true, Ordering::SeqCst);
                    let _ = released.await;
                }
            })),
            docling_recovery: Some(tokio::spawn(async move {
                docling_flag.store(false, Ordering::SeqCst);
            })),
        };

        // Cancel the wait while the source loop is still held.
        let held = tokio::time::timeout(Duration::from_millis(20), handles.wait()).await;
        assert!(held.is_err(), "the source loop is still running");
        assert!(source_done.load(Ordering::SeqCst));
        assert!(
            handles.source_cleanup.is_some(),
            "the cancelled wait keeps the handle owned"
        );

        // The next bounded stop resumes the same join instead of a no-op.
        release.send(()).expect("release the source loop");
        handles.wait().await;
        assert!(handles.source_cleanup.is_none());
        assert!(handles.docling_recovery.is_none());
    }

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
    fn invalid_cleanup_queries_are_http_400_before_io() {
        use context69_contracts::DoclingCleanupQuery;
        for query in [
            DoclingCleanupQuery {
                page: 0,
                ..DoclingCleanupQuery::default()
            },
            DoclingCleanupQuery {
                page_size: 0,
                ..DoclingCleanupQuery::default()
            },
            DoclingCleanupQuery {
                page_size: 101,
                ..DoclingCleanupQuery::default()
            },
            DoclingCleanupQuery {
                remote_task_id: Some("   ".into()),
                ..DoclingCleanupQuery::default()
            },
        ] {
            let error = validate_cleanup_query(&query).unwrap_err();
            assert_eq!(
                crate::domain_errors::status_for_error(&error),
                axum::http::StatusCode::BAD_REQUEST
            );
        }
        assert!(
            validate_cleanup_query(&DoclingCleanupQuery {
                page_size: 20,
                remote_task_id: Some("owned".into()),
                ..DoclingCleanupQuery::default()
            })
            .is_ok()
        );
    }

    #[test]
    fn non_admin_actor_is_forbidden_before_any_cleanup_read() {
        let actor = UserRecord {
            id: 1,
            login_name: "plain".into(),
            display_name: "Plain".into(),
            password_hash: "unused".into(),
            is_admin: false,
            disabled_at: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let error = crate::services::auth::require_admin(&actor).unwrap_err();
        assert_eq!(
            crate::domain_errors::status_for_error(&error),
            axum::http::StatusCode::FORBIDDEN
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
            reason_code: Some(context69_contracts::DoclingCleanupReasonCode::Unspecified),
            attempt_count: 1,
            next_attempt_at: chrono::Utc::now(),
            cleanup_deadline_at: chrono::Utc::now(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            finished_at: None,
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

    #[test]
    fn cleanup_reason_codes_map_only_exact_worker_diagnostics() {
        use context69_contracts::DoclingCleanupReasonCode as Code;
        let reason = docling_cleanup_reason_code;
        assert_eq!(
            reason(
                "unsupported",
                None,
                Some("Submit endpoint unavailable; remote termination unconfirmed")
            ),
            Some(Code::MissingSubmitEndpoint)
        );
        assert_eq!(
            reason(
                "unsupported",
                None,
                Some("Remote cancellation requires unsupported authentication")
            ),
            Some(Code::AuthenticationFailed)
        );
        assert_eq!(
            reason(
                "unsupported",
                None,
                Some("Remote observation requires unsupported authentication")
            ),
            Some(Code::AuthenticationFailed)
        );
        assert_eq!(
            reason(
                "failed",
                None,
                Some("Cleanup deadline exceeded; remote termination unconfirmed")
            ),
            Some(Code::DeadlineExceeded)
        );
        assert_eq!(
            reason(
                "done",
                Some("not_found"),
                Some("Remote task not found; termination not confirmed")
            ),
            Some(Code::RemoteNotFound)
        );
        assert_eq!(
            reason("done", Some("not_found"), None),
            Some(Code::RemoteNotFound)
        );
        assert_eq!(
            reason("awaiting_terminal", None, None),
            Some(Code::AwaitingRemoteTerminal)
        );
        assert_eq!(
            reason(
                "awaiting_terminal",
                None,
                Some("Remote terminal state was not confirmed")
            ),
            Some(Code::AwaitingRemoteTerminal)
        );
        assert_eq!(reason("unsupported", None, None), Some(Code::Unspecified));
        assert_eq!(
            reason("failed", None, Some("some future worker message")),
            Some(Code::Unspecified)
        );
        assert_eq!(reason("pending", None, None), None);
        assert_eq!(reason("cancelling", None, None), None);
        assert_eq!(reason("done", None, None), None);
        assert_eq!(reason("done", Some("cancelled"), None), None);
        assert_eq!(reason("done", Some("success"), None), None);
    }

    #[test]
    fn cleanup_reason_mapping_never_matches_substrings_or_urls() {
        use context69_contracts::DoclingCleanupReasonCode as Code;
        assert_eq!(
            docling_cleanup_reason_code(
                "pending",
                None,
                Some("prefix Submit endpoint unavailable; remote termination unconfirmed")
            ),
            Some(Code::Unspecified)
        );
        assert_eq!(
            docling_cleanup_reason_code("pending", None, Some("http://127.0.0.1:5001/v1/tasks/x")),
            Some(Code::Unspecified)
        );
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
            .map(|row| {
                let reason_code = docling_cleanup_reason_code(
                    &row.status,
                    row.outcome.as_deref(),
                    row.last_error.as_deref(),
                );
                row.into_cleanup_task(reason_code)
            })
            .collect();
        let remote = self
            .library()
            .list_docling_remote_tasks(query.limit, query.offset)
            .await;
        Ok(remote_tasks_with_cleanup(remote, cleanup))
    }

    /// Genuine admin page over the persisted local cleanup outbox.
    ///
    /// Non-admin callers are denied before any query. Filters are exact matches
    /// on persisted columns and the pagination total is the real server total,
    /// so a page beyond the legacy 100-row window stays reachable.
    pub async fn admin_docling_cleanup_page(
        &self,
        actor: &UserRecord,
        query: &DoclingCleanupQuery,
    ) -> Result<DoclingCleanupPageResponse> {
        crate::services::auth::require_admin(actor)?;
        validate_cleanup_query(query)?;
        let page = OffsetPageQuery {
            page: query.page,
            page_size: query.page_size,
        };
        let offset = page.offset().map_err(|error| {
            crate::domain_errors::DomainError::invalid_argument(error.to_string())
        })?;
        let status = query.status.map(|value| value.as_str());
        let outcome = query.outcome.map(|value| value.as_str());
        let remote_task_id = query.normalized_remote_task_id();
        let total = self
            .db()
            .count_docling_cleanup(status, outcome, remote_task_id)
            .await?;
        let rows = if total == 0 {
            Vec::new()
        } else {
            self.db()
                .list_docling_cleanup_page(
                    status,
                    outcome,
                    remote_task_id,
                    i64::from(query.page_size),
                    offset,
                )
                .await?
        };
        let items = rows
            .into_iter()
            .map(|row| {
                let reason_code = docling_cleanup_reason_code(
                    &row.status,
                    row.outcome.as_deref(),
                    row.last_error.as_deref(),
                );
                row.into_cleanup_task(reason_code)
            })
            .collect();
        let total = u64::try_from(total).map_err(|error| {
            crate::domain_errors::DomainError::invalid_argument(error.to_string())
        })?;
        let pagination =
            Pagination::try_new(query.page, query.page_size, total).map_err(|error| {
                crate::domain_errors::DomainError::invalid_argument(error.to_string())
            })?;
        Ok(DoclingCleanupPageResponse { items, pagination })
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
