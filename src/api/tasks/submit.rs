//! Task enqueue handlers: the group-scoped batch endpoints, the generic
//! `POST /v1/tasks` submit, and the vector-index rebuild.

use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
    response::Response,
};
use context69_contracts::{
    ApiErrorResponse, DeleteBatchRequest, FileBatchRequest, TaskRef, TaskSubmitRequest,
    TextBatchRequest, UrlBatchRequest,
};
use serde_json::json;

use super::{
    ApiState, CurrentUser, idempotency_key, managed_group, submit_task_request, task_error,
};
use crate::{contracts::TaskKind, services::tasks::TaskSubmission};

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
        Err(response) => return *response,
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
        Err(response) => return *response,
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
        Err(response) => return *response,
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
        Err(response) => return *response,
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
            Err(response) => return *response,
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

async fn submit(state: &ApiState, request: TaskSubmission) -> Response {
    submit_task_request(state, request).await
}
