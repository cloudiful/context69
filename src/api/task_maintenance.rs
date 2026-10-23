use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};

use super::{ApiState, auth::CurrentUser};

#[utoipa::path(
    post,
    path = "/v1/admin/tasks/cancel-active",
    responses(
        (status = 200, description = "Cancelled all active tasks", body = crate::contracts::CancelActiveTasksResponse),
        (status = 403, description = "Admin access required", body = context69_contracts::ApiErrorResponse)
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
        Err(error) => super::error_mapping::task_maintenance_error_response(error),
    }
}

#[utoipa::path(
    get,
    path = "/v1/admin/docling/tasks",
    params(crate::contracts::DoclingRemoteTasksQuery),
    responses(
        (status = 200, description = "Bounded page of remote Docling task identities", body = crate::contracts::DoclingRemoteTasksResponse),
        (status = 400, description = "Invalid limit or offset", body = context69_contracts::ApiErrorResponse),
        (status = 403, description = "Admin access required", body = context69_contracts::ApiErrorResponse)
    )
)]
pub(crate) async fn list_docling_remote_tasks(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Query(query): Query<crate::contracts::DoclingRemoteTasksQuery>,
) -> Response {
    match state
        .app
        .tasks
        .admin_docling_remote_tasks(&session.user, &query)
        .await
    {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(error) => super::error_mapping::task_maintenance_error_response(error),
    }
}

#[utoipa::path(
    get,
    path = "/v1/admin/docling/cleanup",
    params(crate::contracts::DoclingCleanupQuery),
    responses(
        (status = 200, description = "Filtered page of persisted local Docling cleanup rows", body = crate::contracts::DoclingCleanupPageResponse),
        (status = 400, description = "Invalid page, filter enum, or blank task id", body = context69_contracts::ApiErrorResponse),
        (status = 403, description = "Admin access required", body = context69_contracts::ApiErrorResponse)
    )
)]
pub(crate) async fn list_docling_cleanup(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Query(query): Query<crate::contracts::DoclingCleanupQuery>,
) -> Response {
    match state
        .app
        .tasks
        .admin_docling_cleanup_page(&session.user, &query)
        .await
    {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(error) => super::error_mapping::task_maintenance_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/v1/admin/docling/cleanup",
    responses(
        (status = 200, description = "Eligible owned cleanup scheduled", body = crate::contracts::DoclingCleanupResponse),
        (status = 403, description = "Admin access required", body = context69_contracts::ApiErrorResponse)
    )
)]
pub(crate) async fn wake_docling_cleanup(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
) -> Response {
    match state.app.tasks.admin_docling_cleanup(&session.user).await {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(error) => super::error_mapping::task_maintenance_error_response(error),
    }
}
