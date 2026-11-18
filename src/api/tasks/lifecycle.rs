//! Task lifecycle handlers: reads (`get_task`, `list_tasks`,
//! `list_task_items`), retry/resume, trash/restore/delete, cancel, and the
//! history clear.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use context69_contracts::tasks::TaskSelectionQuery;
use context69_contracts::{
    ApiErrorResponse, CanonicalTaskListQuery, ClearTaskHistoryRequest, TaskBatchLifecycleRequest,
    TaskBatchRerunRequest, TaskBatchRetryRequest, TaskBatchTrashRequest, TaskItemsQuery,
    TaskListQuery,
};
use uuid::Uuid;

use super::{ApiState, CurrentUser, task_error};

#[utoipa::path(get, path = "/v1/tasks/{task_id}", params(("task_id" = Uuid, Path)), responses((status = 200, description = "Task owned by or shared with the caller", body = crate::contracts::TaskResponse), (status = 404, description = "Task not found", body = ApiErrorResponse)))]
pub(crate) async fn get_task(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(task_id): Path<Uuid>,
) -> Response {
    if let Err(response) = super::authorize_task(&state, &session, task_id).await {
        return response;
    }
    match state.app.tasks.get(task_id, session.user.id).await {
        Ok(task) => (StatusCode::OK, Json(task)).into_response(),
        Err(error) => task_error(error),
    }
}

#[utoipa::path(get, path = "/v1/tasks", params(CanonicalTaskListQuery), responses((status = 200, description = "Page of tasks visible to the caller", body = crate::contracts::TaskPageResponse), (status = 400, description = "Invalid task list query", body = ApiErrorResponse)))]
pub(crate) async fn list_tasks(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Query(query): Query<CanonicalTaskListQuery>,
) -> Response {
    if let Err(error) =
        crate::authorization::require_unrestricted(session.token_group_scope.as_ref())
    {
        return task_error(error);
    }
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

#[utoipa::path(get, path = "/v1/tasks/selection", params(TaskSelectionQuery), responses((status = 200, description = "Snapshot of task ids matching the current filters", body = crate::contracts::tasks::TaskSelectionResponse), (status = 400, description = "Invalid task selection query", body = ApiErrorResponse)))]
pub(crate) async fn list_task_selection(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Query(query): Query<TaskSelectionQuery>,
) -> Response {
    if let Err(error) =
        crate::authorization::require_unrestricted(session.token_group_scope.as_ref())
    {
        return task_error(error);
    }
    // Read-only snapshot of every task matching the current filters. The
    // captured ids freeze the scope of a later bulk action; the per-task
    // endpoints stay authoritative for authorization and lifecycle.
    match state.app.tasks.selection(session.user.id, &query).await {
        Ok(selection) => (StatusCode::OK, Json(selection)).into_response(),
        Err(error) => task_error(error),
    }
}

#[utoipa::path(get, path = "/v1/tasks/{task_id}/items", params(("task_id" = Uuid, Path), TaskItemsQuery), responses((status = 200, description = "Page of task items", body = crate::contracts::TaskItemsResponse)))]
pub(crate) async fn list_task_items(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(task_id): Path<Uuid>,
    Query(query): Query<TaskItemsQuery>,
) -> Response {
    if let Err(response) = super::authorize_task(&state, &session, task_id).await {
        return response;
    }
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

#[utoipa::path(get, path = "/v1/tasks/{task_id}/diagnose", params(("task_id" = Uuid, Path)), responses((status = 200, description = "Task diagnosis report", body = crate::contracts::TaskDiagnoseResponse), (status = 404, description = "Task not found", body = ApiErrorResponse)))]
pub(crate) async fn diagnose_task(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(task_id): Path<Uuid>,
) -> Response {
    if let Err(response) = super::authorize_task(&state, &session, task_id).await {
        return response;
    }
    // Same task-detail authorization as `get_task`: the task must belong to the
    // caller or to a group they inherit, so a foreign task is `not_found`.
    match state.app.tasks.diagnose(task_id, session.user.id).await {
        Ok(diagnose) => (StatusCode::OK, Json(diagnose)).into_response(),
        Err(error) => task_error(error),
    }
}

#[utoipa::path(post, path = "/v1/tasks/{task_id}/retry", params(("task_id" = Uuid, Path)), responses((status = 202, description = "Task retry accepted", body = crate::contracts::TaskRetryResponse), (status = 409, description = "Task cannot be retried in its current state", body = ApiErrorResponse)))]
pub(crate) async fn retry_task(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(task_id): Path<Uuid>,
) -> Response {
    if let Err(response) = super::authorize_task(&state, &session, task_id).await {
        return response;
    }
    match state.app.tasks.retry(task_id, session.user.id).await {
        Ok(task) => (StatusCode::ACCEPTED, Json(task)).into_response(),
        Err(error) => task_error(error),
    }
}

#[utoipa::path(post, path = "/v1/tasks/batch/retry", request_body = TaskBatchRetryRequest, responses((status = 200, description = "Ordered per-task retry outcomes", body = crate::contracts::TaskBatchRetryResponse), (status = 400, description = "Invalid batch retry request", body = ApiErrorResponse)))]
pub(crate) async fn batch_retry_tasks(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Json(request): Json<TaskBatchRetryRequest>,
) -> Response {
    if let Err(error) =
        crate::authorization::require_unrestricted(session.token_group_scope.as_ref())
    {
        return task_error(error);
    }
    // One ordered outcome per requested id; per-item authorization and
    // lifecycle stay inside the existing in-place primitives.
    match state.app.tasks.batch_retry(session.user.id, &request).await {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(error) => task_error(error),
    }
}

#[utoipa::path(post, path = "/v1/tasks/{task_id}/rerun", params(("task_id" = Uuid, Path)), responses((status = 202, description = "Task rerun accepted", body = crate::contracts::RerunTaskResponse), (status = 400, description = "Invalid rerun request", body = ApiErrorResponse), (status = 404, description = "Task not found", body = ApiErrorResponse), (status = 409, description = "Task cannot be rerun in its current state", body = ApiErrorResponse)))]
pub(crate) async fn rerun_task(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(task_id): Path<Uuid>,
) -> Response {
    if let Err(response) = super::authorize_task(&state, &session, task_id).await {
        return response;
    }
    // Routing: a cancelled submission reopens its own items and keeps its id,
    // a failed task keeps the documented fresh-parent rerun, and a succeeded
    // file, url, or text task is rerun from its current source.
    match state.app.tasks.rerun(task_id, session.user.id).await {
        Ok(task) => (StatusCode::ACCEPTED, Json(task)).into_response(),
        Err(error) => task_error(error),
    }
}

#[utoipa::path(post, path = "/v1/tasks/batch/rerun", request_body = TaskBatchRerunRequest, responses((status = 200, description = "Ordered per-task rerun outcomes", body = crate::contracts::TaskBatchRerunResponse), (status = 400, description = "Invalid batch rerun request", body = ApiErrorResponse)))]
pub(crate) async fn batch_rerun_tasks(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Json(request): Json<TaskBatchRerunRequest>,
) -> Response {
    if let Err(error) =
        crate::authorization::require_unrestricted(session.token_group_scope.as_ref())
    {
        return task_error(error);
    }
    // One ordered outcome per requested id; only a succeeded file, url, or text
    // parent is rerun, and every other id is skipped with a reason.
    match state.app.tasks.batch_rerun(session.user.id, &request).await {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(error) => task_error(error),
    }
}

#[utoipa::path(post, path = "/v1/tasks/{task_id}/pause", params(("task_id" = Uuid, Path)), responses((status = 200, description = "Paused task", body = crate::contracts::TaskResponse), (status = 403, description = "Insufficient permissions", body = ApiErrorResponse), (status = 404, description = "Task not found", body = ApiErrorResponse), (status = 409, description = "Task cannot be paused in its current state", body = ApiErrorResponse)))]
pub(crate) async fn pause_task(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(task_id): Path<Uuid>,
) -> Response {
    if let Err(response) = super::authorize_task(&state, &session, task_id).await {
        return response;
    }
    match state.app.tasks.pause_task(task_id, session.user.id).await {
        Ok(task) => (StatusCode::OK, Json(task)).into_response(),
        Err(error) => task_error(error),
    }
}

#[utoipa::path(post, path = "/v1/tasks/{task_id}/resume", params(("task_id" = Uuid, Path)), responses((status = 200, description = "Resumed task", body = crate::contracts::TaskResponse), (status = 403, description = "Insufficient permissions", body = ApiErrorResponse), (status = 404, description = "Task not found", body = ApiErrorResponse), (status = 409, description = "Task cannot be resumed in its current state", body = ApiErrorResponse)))]
pub(crate) async fn resume_task(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(task_id): Path<Uuid>,
) -> Response {
    if let Err(response) = super::authorize_task(&state, &session, task_id).await {
        return response;
    }
    match state.app.tasks.resume_task(task_id, session.user.id).await {
        Ok(task) => (StatusCode::OK, Json(task)).into_response(),
        Err(error) => task_error(error),
    }
}

#[utoipa::path(post, path = "/v1/tasks/{task_id}/cancel", params(("task_id" = Uuid, Path)), responses((status = 204, description = "Task cancellation accepted"), (status = 404, description = "Task not found", body = ApiErrorResponse)))]
pub(crate) async fn cancel_task(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(task_id): Path<Uuid>,
) -> Response {
    if let Err(response) = super::authorize_task(&state, &session, task_id).await {
        return response;
    }
    match state.app.tasks.cancel(task_id, session.user.id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => task_error(error),
    }
}

#[utoipa::path(post, path = "/v1/tasks/{task_id}/trash", params(("task_id" = Uuid, Path)), responses((status = 200, description = "Trashed task", body = crate::contracts::TaskResponse), (status = 403, description = "Insufficient permissions", body = ApiErrorResponse), (status = 404, description = "Task not found", body = ApiErrorResponse), (status = 409, description = "Task cannot be trashed in its current state", body = ApiErrorResponse)))]
pub(crate) async fn trash_task(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(task_id): Path<Uuid>,
) -> Response {
    if let Err(response) = super::authorize_task(&state, &session, task_id).await {
        return response;
    }
    match state.app.tasks.trash(task_id, session.user.id).await {
        Ok(task) => (StatusCode::OK, Json(task)).into_response(),
        Err(error) => task_error(error),
    }
}

#[utoipa::path(post, path = "/v1/tasks/{task_id}/restore", params(("task_id" = Uuid, Path)), responses((status = 200, description = "Restored task", body = crate::contracts::TaskResponse), (status = 403, description = "Insufficient permissions", body = ApiErrorResponse), (status = 404, description = "Task not found", body = ApiErrorResponse)))]
pub(crate) async fn restore_task(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(task_id): Path<Uuid>,
) -> Response {
    if let Err(response) = super::authorize_task(&state, &session, task_id).await {
        return response;
    }
    match state.app.tasks.restore(task_id, session.user.id).await {
        Ok(task) => (StatusCode::OK, Json(task)).into_response(),
        Err(error) => task_error(error),
    }
}

#[utoipa::path(post, path = "/v1/tasks/batch/cancel", request_body = TaskBatchLifecycleRequest, responses((status = 200, description = "Ordered per-task cancel outcomes", body = crate::contracts::TaskBatchLifecycleResponse), (status = 400, description = "Invalid batch cancel request", body = ApiErrorResponse)))]
pub(crate) async fn batch_cancel_tasks(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Json(request): Json<TaskBatchLifecycleRequest>,
) -> Response {
    if let Err(error) =
        crate::authorization::require_unrestricted(session.token_group_scope.as_ref())
    {
        return task_error(error);
    }
    // One ordered outcome per requested id; the set is authorized and mutated
    // by bounded batch statements, never by a per-id transaction.
    match state
        .app
        .tasks
        .batch_cancel(session.user.id, &request)
        .await
    {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(error) => task_error(error),
    }
}

#[utoipa::path(post, path = "/v1/tasks/batch/trash", request_body = TaskBatchTrashRequest, responses((status = 200, description = "Ordered per-task trash outcomes", body = crate::contracts::TaskBatchLifecycleResponse), (status = 400, description = "Invalid batch trash request", body = ApiErrorResponse)))]
pub(crate) async fn batch_trash_tasks(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Json(request): Json<TaskBatchTrashRequest>,
) -> Response {
    if let Err(error) =
        crate::authorization::require_unrestricted(session.token_group_scope.as_ref())
    {
        return task_error(error);
    }
    // A caller that opts into `cancel_active` cancels the active rows in the
    // same transaction before their rows are recycled; the default keeps the
    // terminal-only contract and refuses an active row unmoved.
    match state.app.tasks.batch_trash(session.user.id, &request).await {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(error) => task_error(error),
    }
}

#[utoipa::path(post, path = "/v1/tasks/batch/restore", request_body = TaskBatchLifecycleRequest, responses((status = 200, description = "Ordered per-task restore outcomes", body = crate::contracts::TaskBatchLifecycleResponse), (status = 400, description = "Invalid batch restore request", body = ApiErrorResponse)))]
pub(crate) async fn batch_restore_tasks(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Json(request): Json<TaskBatchLifecycleRequest>,
) -> Response {
    if let Err(error) =
        crate::authorization::require_unrestricted(session.token_group_scope.as_ref())
    {
        return task_error(error);
    }
    match state
        .app
        .tasks
        .batch_restore(session.user.id, &request)
        .await
    {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(error) => task_error(error),
    }
}

#[utoipa::path(post, path = "/v1/tasks/batch/purge", request_body = TaskBatchLifecycleRequest, responses((status = 200, description = "Ordered per-task purge outcomes", body = crate::contracts::TaskBatchLifecycleResponse), (status = 400, description = "Invalid batch purge request", body = ApiErrorResponse)))]
pub(crate) async fn batch_purge_tasks(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Json(request): Json<TaskBatchLifecycleRequest>,
) -> Response {
    if let Err(error) =
        crate::authorization::require_unrestricted(session.token_group_scope.as_ref())
    {
        return task_error(error);
    }
    // Only trashed rows are removed; the recycle bin stays the only path to
    // permanent deletion.
    match state.app.tasks.batch_purge(session.user.id, &request).await {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(error) => task_error(error),
    }
}

#[utoipa::path(post, path = "/v1/tasks/clear", request_body = ClearTaskHistoryRequest, responses((status = 200, description = "Cleared task history view", body = crate::contracts::ClearTaskHistoryResponse), (status = 400, description = "Invalid task history clear request", body = ApiErrorResponse)))]
pub(crate) async fn clear_task_history(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Json(request): Json<ClearTaskHistoryRequest>,
) -> Response {
    if let Err(error) =
        crate::authorization::require_unrestricted(session.token_group_scope.as_ref())
    {
        return task_error(error);
    }
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

#[utoipa::path(delete, path = "/v1/tasks/{task_id}", params(("task_id" = Uuid, Path)), responses((status = 204, description = "Task permanently removed"), (status = 403, description = "Insufficient permissions", body = ApiErrorResponse), (status = 404, description = "Task not found", body = ApiErrorResponse), (status = 409, description = "Task cannot be deleted in its current state", body = ApiErrorResponse)))]
pub(crate) async fn delete_task(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(task_id): Path<Uuid>,
) -> Response {
    if let Err(response) = super::authorize_task(&state, &session, task_id).await {
        return response;
    }
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
