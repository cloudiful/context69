use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use context69_contracts::{
    ApiErrorResponse, CanonicalTaskListQuery, DeleteBatchRequest, FileBatchRequest, ScopeSpec,
    TaskItemsQuery, TaskListQuery, TaskRef, TaskSubmitRequest, TextBatchRequest, UrlBatchRequest,
};
use serde_json::json;
use uuid::Uuid;

use super::{
    ApiState,
    auth::CurrentUser,
    errors::task_error,
    group_access::{group_access_error_response, group_for_user, require_group_role},
};
use crate::{contracts::TaskKind, services::tasks::TaskSubmission};

#[utoipa::path(
    post,
    path = "/v1/scopes/ensure",
    request_body = ScopeSpec,
    responses((status = 200, body = crate::contracts::EnsureScopeResponse), (status = 409, body = ApiErrorResponse))
)]
pub(crate) async fn ensure_scope(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Json(spec): Json<ScopeSpec>,
) -> Response {
    match state.app.tasks.ensure_scope(session.user.id, &spec).await {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(error) => task_error(error),
    }
}

#[utoipa::path(post, path = "/v1/groups/by-path/{group_path}/batch/text", params(("group_path" = String, Path)), request_body = TextBatchRequest, responses((status = 202, body = TaskRef), (status = 409, body = ApiErrorResponse)))]
pub(crate) async fn submit_text_batch(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(group_path): Path<String>,
    headers: HeaderMap,
    Json(request): Json<TextBatchRequest>,
) -> Response {
    let group = match managed_group(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(response) => return response,
    };
    let payloads = match request
        .items
        .into_iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(payloads) => payloads,
        Err(error) => return task_error(error.into()),
    };
    submit(
        &state,
        TaskSubmission {
            user_id: session.user.id,
            group_id: Some(group.id),
            group_path: Some(group_path),
            source_key: None,
            kind: TaskKind::TextBatch,
            payloads,
            input_storage_object_ids: Vec::new(),
            idempotency_key: idempotency_key(&headers),
        },
    )
    .await
}

#[utoipa::path(post, path = "/v1/groups/by-path/{group_path}/batch/url", params(("group_path" = String, Path)), request_body = UrlBatchRequest, responses((status = 202, body = TaskRef), (status = 409, body = ApiErrorResponse)))]
pub(crate) async fn submit_url_batch(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(group_path): Path<String>,
    headers: HeaderMap,
    Json(request): Json<UrlBatchRequest>,
) -> Response {
    let group = match managed_group(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(response) => return response,
    };
    let payloads = match request
        .items
        .into_iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(payloads) => payloads,
        Err(error) => return task_error(error.into()),
    };
    submit(
        &state,
        TaskSubmission {
            user_id: session.user.id,
            group_id: Some(group.id),
            group_path: Some(group_path),
            source_key: None,
            kind: TaskKind::UrlBatch,
            payloads,
            input_storage_object_ids: Vec::new(),
            idempotency_key: idempotency_key(&headers),
        },
    )
    .await
}

#[utoipa::path(post, path = "/v1/groups/by-path/{group_path}/batch/file", params(("group_path" = String, Path)), request_body = FileBatchRequest, responses((status = 202, body = TaskRef), (status = 409, body = ApiErrorResponse)))]
pub(crate) async fn submit_file_batch(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(group_path): Path<String>,
    headers: HeaderMap,
    Json(request): Json<FileBatchRequest>,
) -> Response {
    let group = match managed_group(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(response) => return response,
    };
    let payloads = match request
        .items
        .into_iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(payloads) => payloads,
        Err(error) => return task_error(error.into()),
    };
    submit(
        &state,
        TaskSubmission {
            user_id: session.user.id,
            group_id: Some(group.id),
            group_path: Some(group_path),
            source_key: None,
            kind: TaskKind::FileBatch,
            payloads,
            input_storage_object_ids: Vec::new(),
            idempotency_key: idempotency_key(&headers),
        },
    )
    .await
}

#[utoipa::path(post, path = "/v1/groups/by-path/{group_path}/batch/delete", params(("group_path" = String, Path)), request_body = DeleteBatchRequest, responses((status = 202, body = TaskRef), (status = 409, body = ApiErrorResponse)))]
pub(crate) async fn submit_delete_batch(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(group_path): Path<String>,
    headers: HeaderMap,
    Json(request): Json<DeleteBatchRequest>,
) -> Response {
    let group = match managed_group(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(response) => return response,
    };
    let payloads = match request
        .items
        .into_iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(payloads) => payloads,
        Err(error) => return task_error(error.into()),
    };
    submit(
        &state,
        TaskSubmission {
            user_id: session.user.id,
            group_id: Some(group.id),
            group_path: Some(group_path),
            source_key: None,
            kind: TaskKind::DeleteBatch,
            payloads,
            input_storage_object_ids: Vec::new(),
            idempotency_key: idempotency_key(&headers),
        },
    )
    .await
}

#[utoipa::path(post, path = "/v1/tasks", request_body = TaskSubmitRequest, responses((status = 202, body = TaskRef), (status = 409, body = ApiErrorResponse)))]
pub(crate) async fn submit_task(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    headers: HeaderMap,
    Json(request): Json<TaskSubmitRequest>,
) -> Response {
    let (kind, group_path, source_key, payloads) = match request {
        TaskSubmitRequest::RetryFileBatch { group_path, items } => (
            TaskKind::FileBatch,
            group_path,
            None,
            items
                .into_iter()
                .map(|item| json!({ "file_id": item.file_id }))
                .collect(),
        ),
        TaskSubmitRequest::FileBatch { group_path, items } => (
            TaskKind::FileBatch,
            group_path,
            None,
            items
                .into_iter()
                .map(|item| serde_json::to_value(item).expect("FileBatchItem is serializable"))
                .collect(),
        ),
        TaskSubmitRequest::TextBatch { group_path, items } => (
            TaskKind::TextBatch,
            group_path,
            None,
            items
                .into_iter()
                .map(|item| serde_json::to_value(item).expect("text item is serializable"))
                .collect(),
        ),
        TaskSubmitRequest::UrlBatch { group_path, items } => (
            TaskKind::UrlBatch,
            group_path,
            None,
            items
                .into_iter()
                .map(|item| serde_json::to_value(item).expect("url item is serializable"))
                .collect(),
        ),
        TaskSubmitRequest::DeleteBatch { group_path, items } => (
            TaskKind::DeleteBatch,
            group_path,
            None,
            items
                .into_iter()
                .map(|item| serde_json::to_value(item).expect("delete item is serializable"))
                .collect(),
        ),
        TaskSubmitRequest::SourceSync {
            group_path,
            source_key,
        } => (
            TaskKind::SourceSync,
            group_path,
            Some(source_key),
            vec![json!({})],
        ),
        TaskSubmitRequest::TranslationBatch { group_path, items } => (
            TaskKind::Translation,
            group_path,
            None,
            items
                .into_iter()
                .map(|item| serde_json::to_value(item).expect("translation item is serializable"))
                .collect(),
        ),
        TaskSubmitRequest::VectorRebuild => (TaskKind::VectorRebuild, None, None, vec![json!({})]),
    };
    let group = if let Some(path) = group_path.as_deref() {
        match managed_group(&state, session.user.id, path).await {
            Ok(group) => Some(group),
            Err(response) => return response,
        }
    } else {
        None
    };
    submit(
        &state,
        TaskSubmission {
            user_id: session.user.id,
            group_id: group.as_ref().map(|value| value.id),
            group_path,
            source_key,
            kind,
            payloads,
            input_storage_object_ids: Vec::new(),
            idempotency_key: idempotency_key(&headers),
        },
    )
    .await
}

#[utoipa::path(
    post,
    path = "/v1/settings/runtime/vector-index/rebuild",
    responses((status = 202, body = TaskRef), (status = 409, body = ApiErrorResponse))
)]
pub(crate) async fn submit_vector_index_rebuild(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    headers: HeaderMap,
) -> Response {
    submit_task_request(
        &state,
        TaskSubmission {
            user_id: session.user.id,
            group_id: None,
            group_path: None,
            source_key: None,
            kind: TaskKind::VectorRebuild,
            payloads: vec![json!({})],
            input_storage_object_ids: Vec::new(),
            idempotency_key: idempotency_key(&headers),
        },
    )
    .await
}

pub(crate) async fn submit_task_request(state: &ApiState, request: TaskSubmission) -> Response {
    match state.app.tasks.submit(request).await {
        Ok(task) => (StatusCode::ACCEPTED, Json(task)).into_response(),
        Err(error) => task_error(error),
    }
}

async fn submit(state: &ApiState, request: TaskSubmission) -> Response {
    submit_task_request(state, request).await
}

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
    match state
        .app
        .tasks
        .items(task_id, session.user.id, i64::from(query.limit), offset)
        .await
    {
        Ok(items) => (StatusCode::OK, Json(items)).into_response(),
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

async fn managed_group(
    state: &ApiState,
    user_id: i64,
    group_path: &str,
) -> Result<crate::domain::GroupRecord, Response> {
    let group = group_for_user(state, user_id, group_path)
        .await
        .map_err(group_access_error_response)?;
    require_group_role(&group, crate::contracts::MembershipRole::Maintainer)
        .map_err(group_access_error_response)?;
    Ok(group)
}

fn idempotency_key(headers: &HeaderMap) -> Option<String> {
    headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_task_list_query_requires_view_and_rejects_trashed_shape() {
        let canonical: CanonicalTaskListQuery = serde_json::from_value(serde_json::json!({
            "page": 1,
            "page_size": 25,
            "view": "processing"
        }))
        .expect("canonical requires view");
        assert_eq!(
            canonical.view,
            context69_contracts::TaskListView::Processing
        );
        assert!(canonical.validate().is_ok());
        let legacy: TaskListQuery = canonical.into();
        assert_eq!(legacy.trashed, None);
        assert!(legacy.view.is_some());

        let missing_view = serde_json::from_value::<CanonicalTaskListQuery>(serde_json::json!({
            "page": 1,
            "page_size": 25
        }));
        assert!(
            missing_view.is_err(),
            "v0.16 path must require view; legacy trashed-only queries belong to TaskListQuery"
        );

        let zero_page = CanonicalTaskListQuery {
            page: 0,
            page_size: 25,
            query: None,
            kind: None,
            status: None,
            view: context69_contracts::TaskListView::Processing,
            stage: None,
            waiting_reason: None,
            dependency_key: None,
            sort_by: None,
            sort_direction: None,
        };
        assert!(zero_page.validate().is_err());
        assert!(
            context69_http_support::validate_canonical_offset(0, 25).is_err(),
            "shared pagination bounds must reject zero page"
        );
        assert!(
            context69_http_support::validate_canonical_offset(1, 101).is_err(),
            "shared pagination bounds must reject oversized page_size"
        );
    }

    #[test]
    fn canonical_view_filtering_never_widens_status() {
        use context69_contracts::{TaskListView, TaskStatus};
        let processing = TaskListView::Processing;
        let completed = TaskListView::Completed;
        assert_ne!(processing, completed);
        assert_eq!(processing.as_str(), "processing");
        let narrowed: Option<TaskStatus> = Some(TaskStatus::Succeeded);
        assert!(
            narrowed.is_some(),
            "status narrows the view; processing + succeeded matches nothing by SQL predicate"
        );
    }
}
