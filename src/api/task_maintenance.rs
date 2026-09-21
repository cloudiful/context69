use axum::{
    Json,
    extract::State,
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
