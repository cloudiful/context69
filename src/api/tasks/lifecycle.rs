//! Task lifecycle handlers: reads (`get_task`, `list_tasks`,
//! `list_task_items`), retry/resume, trash/restore/delete, cancel, and the
//! history clear.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use context69_contracts::{
    ApiErrorResponse, CanonicalTaskListQuery, ClearTaskHistoryRequest, TaskItemsQuery,
    TaskListQuery,
};
use uuid::Uuid;

use super::{ApiState, CurrentUser, task_error};

#[utoipa::path(get, path = "/v1/tasks/{task_id}", params(("task_id" = Uuid, Path)), responses((status = 200, body = crate::contracts::TaskResponse), (status = 404, body = ApiErrorResponse)))]
pub(crate) async fn get_task(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(task_id): Path<Uuid>,
) -> Response {
    match state.app.tasks.get(task_id, session.user.id).await {
        Ok(task) => (StatusCode::OK, Json(task)).into_response(),
        Err(error) => task_error(error),
    }
}

#[utoipa::path(get, path = "/v1/tasks", params(CanonicalTaskListQuery), responses((status = 200, body = crate::contracts::TaskPageResponse), (status = 400, body = ApiErrorResponse)))]
pub(crate) async fn list_tasks(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Query(query): Query<CanonicalTaskListQuery>,
) -> Response {
    if let Err(error) = query.validate() {
        return task_error(error);
    }
    if let Err(error) =
        context69_http_support::validate_canonical_offset(query.page, query.page_size)
    {
        return task_error(error);
    }
    let legacy: TaskListQuery = query.into();
    match state.app.tasks.list(session.user.id, &legacy).await {
        Ok(tasks) => (StatusCode::OK, Json(tasks)).into_response(),
        Err(error) => task_error(error),
    }
}

#[utoipa::path(get, path = "/v1/tasks/{task_id}/items", params(("task_id" = Uuid, Path), TaskItemsQuery), responses((status = 200, body = crate::contracts::TaskItemsResponse)))]
pub(crate) async fn list_task_items(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(task_id): Path<Uuid>,
    Query(query): Query<TaskItemsQuery>,
) -> Response {
    let offset = query.cursor.as_deref().unwrap_or("0").parse::<i64>();
    let offset = match offset {
        Ok(offset) if offset >= 0 => offset,
        _ => return task_error(anyhow::anyhow!("cursor must be a non-negative integer")),
    };
    let limit = i64::from(query.limit);
    match state
        .app
        .tasks
        .items(task_id, session.user.id, limit, offset, query.status)
        .await
    {
        Ok(items) => (StatusCode::OK, Json(items)).into_response(),
        Err(error) => {
            // Issue #446 P1: list-items 500s previously had no log, leaving
            // only the client-side error. Keep task_id/limit/offset plus
            // the full anyhow chain so slow-SQL/pool failures are diagnosable.
            tracing::error!(
                task_id = %task_id,
                limit,
                offset,
                error = %error,
                chain = ?error.chain().map(ToString::to_string).collect::<Vec<_>>(),
                "list task items failed"
            );
            task_error(error)
        }
    }
}

#[utoipa::path(get, path = "/v1/tasks/{task_id}/diagnose", params(("task_id" = Uuid, Path)), responses((status = 200, body = crate::contracts::TaskDiagnoseResponse), (status = 404, body = ApiErrorResponse)))]
pub(crate) async fn diagnose_task(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(task_id): Path<Uuid>,
) -> Response {
    // Same task-detail authorization as `get_task`: the task must belong to the
    // caller or to a group they inherit, so a foreign task is `not_found`.
    match state.app.tasks.diagnose(task_id, session.user.id).await {
        Ok(diagnose) => (StatusCode::OK, Json(diagnose)).into_response(),
        Err(error) => task_error(error),
    }
}

#[utoipa::path(post, path = "/v1/tasks/{task_id}/retry", params(("task_id" = Uuid, Path)), responses((status = 202, body = crate::contracts::TaskRetryResponse), (status = 409, body = ApiErrorResponse)))]
pub(crate) async fn retry_task(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(task_id): Path<Uuid>,
) -> Response {
    match state.app.tasks.retry(task_id, session.user.id).await {
        Ok(task) => (StatusCode::ACCEPTED, Json(task)).into_response(),
        Err(error) => task_error(error),
    }
}

#[utoipa::path(post, path = "/v1/tasks/{task_id}/rerun", params(("task_id" = Uuid, Path)), responses((status = 202, body = crate::contracts::RerunTaskResponse), (status = 400, body = ApiErrorResponse), (status = 409, body = ApiErrorResponse)))]
pub(crate) async fn rerun_task(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(task_id): Path<Uuid>,
) -> Response {
    // Routing: a cancelled submission reopens its own items and keeps its id,
    // while a failed task keeps the documented fresh-parent rerun.
    match state.app.tasks.rerun(task_id, session.user.id).await {
        Ok(task) => (StatusCode::ACCEPTED, Json(task)).into_response(),
        Err(error) => task_error(error),
    }
}

#[utoipa::path(post, path = "/v1/tasks/{task_id}/cancel", params(("task_id" = Uuid, Path)), responses((status = 204), (status = 404, body = ApiErrorResponse)))]
pub(crate) async fn cancel_task(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(task_id): Path<Uuid>,
) -> Response {
    match state.app.tasks.cancel(task_id, session.user.id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => task_error(error),
    }
}

#[utoipa::path(post, path = "/v1/tasks/{task_id}/trash", params(("task_id" = Uuid, Path)), responses((status = 200, body = crate::contracts::TaskResponse), (status = 403, body = ApiErrorResponse), (status = 404, body = ApiErrorResponse), (status = 409, body = ApiErrorResponse)))]
pub(crate) async fn trash_task(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(task_id): Path<Uuid>,
) -> Response {
    match state.app.tasks.trash(task_id, session.user.id).await {
        Ok(task) => (StatusCode::OK, Json(task)).into_response(),
        Err(error) => task_error(error),
    }
}

#[utoipa::path(post, path = "/v1/tasks/{task_id}/restore", params(("task_id" = Uuid, Path)), responses((status = 200, body = crate::contracts::TaskResponse), (status = 403, body = ApiErrorResponse), (status = 404, body = ApiErrorResponse)))]
pub(crate) async fn restore_task(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(task_id): Path<Uuid>,
) -> Response {
    match state.app.tasks.restore(task_id, session.user.id).await {
        Ok(task) => (StatusCode::OK, Json(task)).into_response(),
        Err(error) => task_error(error),
    }
}

#[utoipa::path(post, path = "/v1/tasks/clear", request_body = ClearTaskHistoryRequest, responses((status = 200, body = crate::contracts::ClearTaskHistoryResponse), (status = 400, body = ApiErrorResponse)))]
pub(crate) async fn clear_task_history(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Json(request): Json<ClearTaskHistoryRequest>,
) -> Response {
    match state
        .app
        .tasks
        .clear_task_history(session.user.id, request.view)
        .await
    {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(error) => task_error(error),
    }
}

#[utoipa::path(delete, path = "/v1/tasks/{task_id}", params(("task_id" = Uuid, Path)), responses((status = 204), (status = 403, body = ApiErrorResponse), (status = 404, body = ApiErrorResponse), (status = 409, body = ApiErrorResponse)))]
pub(crate) async fn delete_task(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(task_id): Path<Uuid>,
) -> Response {
    match state
        .app
        .tasks
        .delete_permanently(task_id, session.user.id)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => task_error(error),
    }
}

#[cfg(test)]
mod tests;
