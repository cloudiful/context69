use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use context69_contracts::{
    ApiErrorResponse, PurgeTasksRequest, RecoverDoclingTaskRequest, RecoverDoclingTaskResponse,
    TaskMaintenanceOverview, UpdateTaskMaintenanceSettingsRequest,
};
use uuid::Uuid;

use super::{ApiState, auth::CurrentUser, errors::error_response};
use crate::services::tasks::TaskMaintenanceError;

#[utoipa::path(
    get,
    path = "/v1/admin/tasks/maintenance",
    responses(
        (status = 200, description = "Task maintenance settings and statistics", body = TaskMaintenanceOverview),
        (status = 403, description = "Admin access required", body = ApiErrorResponse)
    )
)]
pub(crate) async fn get_task_maintenance(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
) -> Response {
    match state
        .app
        .tasks
        .admin_maintenance_overview(&session.user)
        .await
    {
        Ok(overview) => (StatusCode::OK, Json(overview)).into_response(),
        Err(error) => task_maintenance_error_response(error),
    }
}

#[utoipa::path(
    put,
    path = "/v1/admin/tasks/maintenance",
    request_body = UpdateTaskMaintenanceSettingsRequest,
    responses(
        (status = 200, description = "Updated task maintenance settings and statistics", body = TaskMaintenanceOverview),
        (status = 400, description = "Invalid settings payload", body = ApiErrorResponse),
        (status = 403, description = "Admin access required", body = ApiErrorResponse)
    )
)]
pub(crate) async fn update_task_maintenance(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Json(request): Json<UpdateTaskMaintenanceSettingsRequest>,
) -> Response {
    match state
        .app
        .tasks
        .admin_update_maintenance_settings(&session.user, &request)
        .await
    {
        Ok(overview) => (StatusCode::OK, Json(overview)).into_response(),
        Err(error) => task_maintenance_error_response(error),
    }
}

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
    path = "/v1/admin/tasks/purge",
    request_body = PurgeTasksRequest,
    responses(
        (status = 200, description = "Purged task history", body = crate::contracts::PurgeTasksResponse),
        (status = 400, description = "Invalid purge mode", body = ApiErrorResponse),
        (status = 403, description = "Admin access required", body = ApiErrorResponse),
        (status = 409, description = "Active tasks block full history purge", body = ApiErrorResponse)
    )
)]
pub(crate) async fn purge_tasks(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Json(request): Json<PurgeTasksRequest>,
) -> Response {
    match state
        .app
        .tasks
        .admin_purge_tasks(&session.user, request.mode)
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

fn task_maintenance_error_response(error: anyhow::Error) -> Response {
    let message = error.to_string();
    let recovery_error = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<TaskMaintenanceError>());
    let status = if let Some(recovery_error) = recovery_error {
        match recovery_error {
            TaskMaintenanceError::BadRequest(_) => StatusCode::BAD_REQUEST,
            TaskMaintenanceError::Conflict(_) => StatusCode::CONFLICT,
            TaskMaintenanceError::NotFound(_) => StatusCode::NOT_FOUND,
        }
    } else if message.contains("admin access required") {
        StatusCode::FORBIDDEN
    } else if message.contains("must be cancelled") {
        StatusCode::CONFLICT
    } else if message.contains("must be between") {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };
    (status, Json(error_response(status, message))).into_response()
}
