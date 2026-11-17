//! Group-scoped image ingestion, visual search, document image lists, and
//! occurrence unlinking.

use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
    response::{IntoResponse, Response},
};
use context69_contracts::{
    ApiErrorResponse, DocumentImageListResponse, ImageBatchRequest, ImageSearchRequest,
    ImageSearchResponse, TaskRef, UnlinkDocumentImagesRequest, UnlinkDocumentImagesResponse,
};

use super::{
    ApiState, errors::library_management_error_response, group_access::group_access_error_response,
    submit_task_request, tasks::idempotency_key, tasks::managed_group,
};
use crate::api::auth::CurrentUser;
use crate::{contracts::TaskKind, services::tasks::TaskSubmission};

#[utoipa::path(
    post,
    path = "/v1/groups/by-path/{group_path}/batch/image",
    params(("group_path" = String, Path, description = "URL-encoded group path")),
    request_body = ImageBatchRequest,
    responses(
        (status = 202, description = "Image ingestion task accepted", body = TaskRef),
        (status = 409, description = "Idempotency key reuse conflict", body = ApiErrorResponse)
    )
)]
pub(crate) async fn submit_image_batch(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(group_path): Path<String>,
    headers: HeaderMap,
    Json(request): Json<ImageBatchRequest>,
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
        Err(error) => return library_management_error_response(error.into()),
    };
    submit_task_request(
        &state,
        TaskSubmission {
            user_id: session.user.id,
            group_id: Some(group.id),
            group_path: Some(group_path),
            source_key: None,
            kind: TaskKind::ImageBatch,
            payloads,
            input_storage_object_ids: Vec::new(),
            idempotency_key: idempotency_key(&headers),
        },
    )
    .await
}

#[utoipa::path(
    post,
    path = "/v1/groups/by-path/{group_path}/images/search",
    params(("group_path" = String, Path, description = "URL-encoded group path")),
    request_body = ImageSearchRequest,
    responses(
        (status = 200, description = "Authorized image search hits; hydration may return fewer than the requested limit", body = ImageSearchResponse),
        (status = 400, description = "Invalid image search request", body = ApiErrorResponse),
        (status = 403, description = "Insufficient permissions", body = ApiErrorResponse),
        (status = 503, description = "Image search runtime unavailable", body = ApiErrorResponse)
    )
)]
pub(crate) async fn search_images(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(group_path): Path<String>,
    Json(request): Json<ImageSearchRequest>,
) -> Response {
    let (group, scope) = match group_and_scope(&state, session.user.id, &group_path).await {
        Ok(value) => value,
        Err(error) => return group_access_error_response(error),
    };
    match state
        .app
        .library
        .search_images(&group, &scope, request)
        .await
    {
        Ok(response) => Json(response).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(
    get,
    path = "/v1/groups/by-path/{group_path}/documents/{document_id}/images",
    params(
        ("group_path" = String, Path, description = "URL-encoded group path"),
        ("document_id" = i64, Path, description = "Parent document id")
    ),
    responses(
        (status = 200, description = "Bounded current image occurrences of the document", body = DocumentImageListResponse),
        (status = 403, description = "Insufficient permissions", body = ApiErrorResponse),
        (status = 404, description = "Document or group not found", body = ApiErrorResponse)
    )
)]
pub(crate) async fn list_document_images(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_path, document_id)): Path<(String, i64)>,
) -> Response {
    let (group, scope) = match group_and_scope(&state, session.user.id, &group_path).await {
        Ok(value) => value,
        Err(error) => return group_access_error_response(error),
    };
    match state
        .app
        .library
        .list_document_images(group.id, document_id, &scope)
        .await
    {
        Ok(response) => Json(response).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/v1/groups/by-path/{group_path}/documents/images/unlink",
    params(("group_path" = String, Path, description = "URL-encoded group path")),
    request_body = UnlinkDocumentImagesRequest,
    responses(
        (status = 200, description = "Request-ordered per-item unlink outcomes", body = UnlinkDocumentImagesResponse),
        (status = 400, description = "Empty, oversized, or invalid unlink batch", body = ApiErrorResponse),
        (status = 403, description = "Insufficient permissions", body = ApiErrorResponse),
        (status = 404, description = "Group not found", body = ApiErrorResponse)
    )
)]
pub(crate) async fn unlink_document_images(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(group_path): Path<String>,
    Json(request): Json<UnlinkDocumentImagesRequest>,
) -> Response {
    let group = match managed_group(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(response) => return *response,
    };
    match state
        .app
        .library
        .unlink_document_images(group.id, request)
        .await
    {
        Ok(response) => Json(response).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

async fn group_and_scope(
    state: &ApiState,
    user_id: i64,
    group_path: &str,
) -> anyhow::Result<(crate::domain::GroupRecord, crate::domain::AccessScope)> {
    let group = super::group_access::group_for_user(state, user_id, group_path).await?;
    let scope = state
        .app
        .auth
        .access_scope(Some(user_id), Some(group_path.to_string()))
        .await?;
    Ok((group, scope))
}
