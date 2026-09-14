use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use context69_contracts::{
    ApiErrorResponse, QuarantineStaleSubmittingRequest, QuarantineStaleSubmittingResponse,
    QueueDoclingRecoveryRequest, QueueDoclingRecoveryResponse, RecoverDoclingTaskRequest,
    RecoverDoclingTaskResponse,
};
use uuid::Uuid;

use super::{ApiState, auth::CurrentUser};

#[utoipa::path(
    post,
    path = "/v1/admin/tasks/cancel-active",
    responses(
        (status = 200, description = "Cancelled all active tasks", body = crate::contracts::CancelActiveTasksResponse),
        (status = 403, description = "Admin access required", body = ApiErrorResponse)
    )
)]
pub(crate) async fn cancel_active_tasks(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
) -> Response {
    match state
        .app
        .tasks
        .admin_cancel_active_tasks(&session.user)
        .await
    {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(error) => task_maintenance_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/v1/admin/tasks/{task_id}/recover",
    params(("task_id" = Uuid, Path)),
    request_body = RecoverDoclingTaskRequest,
    responses(
        (status = 200, description = "Recovered Docling task with fresh remote job", body = RecoverDoclingTaskResponse),
        (status = 400, description = "Missing or empty reason", body = ApiErrorResponse),
        (status = 403, description = "Admin access required", body = ApiErrorResponse),
        (status = 404, description = "Task not found", body = ApiErrorResponse),
        (status = 409, description = "Task or item is terminal / has an active lease or external job / is not in a Docling stage", body = ApiErrorResponse)
    )
)]
pub(crate) async fn recover_docling_task(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(task_id): Path<Uuid>,
    Json(request): Json<RecoverDoclingTaskRequest>,
) -> Response {
    match state
        .app
        .tasks
        .admin_recover_docling_task(&session.user, task_id, &request)
        .await
    {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(error) => task_maintenance_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/v1/admin/tasks/{task_id}/recover/queue",
    params(("task_id" = Uuid, Path)),
    request_body = QueueDoclingRecoveryRequest,
    responses(
        (status = 200, description = "Requeued Docling task for dispatcher pickup without contacting Docling", body = QueueDoclingRecoveryResponse),
        (status = 400, description = "Missing or empty reason", body = ApiErrorResponse),
        (status = 403, description = "Admin access required", body = ApiErrorResponse),
        (status = 404, description = "Task not found", body = ApiErrorResponse),
        (status = 409, description = "Task or item is terminal / has an active lease or external job / carries an uncertain submitting job / is not in a Docling stage", body = ApiErrorResponse)
    )
)]
pub(crate) async fn queue_docling_recovery(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(task_id): Path<Uuid>,
    Json(request): Json<QueueDoclingRecoveryRequest>,
) -> Response {
    match state
        .app
        .tasks
        .admin_queue_docling_recovery(&session.user, task_id, &request)
        .await
    {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(error) => task_maintenance_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/v1/admin/tasks/quarantine-submitting",
    request_body = QuarantineStaleSubmittingRequest,
    responses(
        (status = 200, description = "Quarantined stale submitting jobs as orphaned with skip counts; dry_run=true returns only eligibility counts with zero writes", body = QuarantineStaleSubmittingResponse),
        (status = 400, description = "Missing or empty reason, or grace_minutes out of bounds", body = ApiErrorResponse),
        (status = 403, description = "Admin access required", body = ApiErrorResponse)
    )
)]
pub(crate) async fn quarantine_stale_submitting(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Json(request): Json<QuarantineStaleSubmittingRequest>,
) -> Response {
    match state
        .app
        .tasks
        .admin_quarantine_stale_submitting(&session.user, &request)
        .await
    {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(error) => task_maintenance_error_response(error),
    }
}

fn task_maintenance_error_response(error: anyhow::Error) -> Response {
    super::error_mapping::task_maintenance_error_response(error)
}
