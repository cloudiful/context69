use anyhow::Result;

use context69_contracts::CancelActiveTasksResponse;

use super::TaskService;
use crate::domain::UserRecord;

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
            .unwrap_or_default();
        dispatcher
            .run(release_service.library().clone(), shutdown)
            .await;
    });
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
