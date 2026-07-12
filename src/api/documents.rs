use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use uuid::Uuid;

use super::{
    ApiState,
    auth::CurrentUser,
    errors::library_management_error_response,
    group_access::{group_access_error_response, group_for_user, require_group_role},
};
use crate::contracts::{
    BatchGetDocumentsRequest, BatchGetDocumentsResponse, CreateMetadataIndexRequest, DocumentKey,
    DocumentLookupQuery, DocumentQueryRequest, DocumentQueryResponse, DocumentResponse,
    MembershipRole, MetadataIndexResponse, UpdateMetadataIndexRequest,
};

async fn group_and_scope(
    state: &ApiState,
    user_id: i64,
    group_path: &str,
) -> anyhow::Result<(crate::domain::GroupRecord, crate::domain::AccessScope)> {
    let group = group_for_user(state, user_id, group_path).await?;
    let scope = state
        .app
        .auth
        .access_scope(Some(user_id), Some(group_path.to_string()))
        .await?;
    Ok((group, scope))
}

#[utoipa::path(post, path = "/v1/groups/by-path/{group_path}/documents/query", params(("group_path" = String, Path)),
    request_body = DocumentQueryRequest, responses((status = 200, body = DocumentQueryResponse), (status = 400, body = crate::contracts::ApiErrorResponse)))]
pub(crate) async fn query_group_documents(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(group_path): Path<String>,
    Json(request): Json<DocumentQueryRequest>,
) -> impl IntoResponse {
    let (group, scope) = match group_and_scope(&state, session.user.id, &group_path).await {
        Ok(value) => value,
        Err(error) => return group_access_error_response(error),
    };
    match state
        .app
        .document_store
        .query(group.id, &request, &scope)
        .await
    {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(get, path = "/v1/groups/by-path/{group_path}/documents/by-external-id", params(("group_path" = String, Path), DocumentLookupQuery),
    responses((status = 200, body = DocumentResponse), (status = 404, body = crate::contracts::ApiErrorResponse)))]
pub(crate) async fn get_group_document_by_key(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(group_path): Path<String>,
    Query(query): Query<DocumentLookupQuery>,
) -> impl IntoResponse {
    let (group, scope) = match group_and_scope(&state, session.user.id, &group_path).await {
        Ok(value) => value,
        Err(error) => return group_access_error_response(error),
    };
    let key = DocumentKey {
        source_key: query.source_key,
        external_id: query.external_id,
    };
    match state
        .app
        .document_store
        .get_by_key(group.id, &key, &scope)
        .await
    {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(post, path = "/v1/groups/by-path/{group_path}/documents/batch-get", params(("group_path" = String, Path)),
    request_body = BatchGetDocumentsRequest, responses((status = 200, body = BatchGetDocumentsResponse)))]
pub(crate) async fn batch_get_group_documents(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(group_path): Path<String>,
    Json(request): Json<BatchGetDocumentsRequest>,
) -> impl IntoResponse {
    let (group, scope) = match group_and_scope(&state, session.user.id, &group_path).await {
        Ok(value) => value,
        Err(error) => return group_access_error_response(error),
    };
    match state
        .app
        .document_store
        .batch_get(group.id, &request.keys, &scope)
        .await
    {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(delete, path = "/v1/groups/by-path/{group_path}/documents/by-external-id", params(("group_path" = String, Path), DocumentLookupQuery), responses((status = 204), (status = 404)))]
pub(crate) async fn delete_group_document_by_key(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(group_path): Path<String>,
    Query(query): Query<DocumentLookupQuery>,
) -> impl IntoResponse {
    let group = match group_for_user(&state, session.user.id, &group_path).await {
        Ok(value) => value,
        Err(error) => return group_access_error_response(error),
    };
    if let Err(error) = require_group_role(&group, MembershipRole::Maintainer) {
        return group_access_error_response(error);
    }
    let key = DocumentKey {
        source_key: query.source_key,
        external_id: query.external_id,
    };
    match state.app.document_store.delete_by_key(&group, &key).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[derive(serde::Deserialize, utoipa::IntoParams)]
pub(crate) struct SourceKeyQuery {
    source_key: String,
}

#[utoipa::path(get, path = "/v1/groups/by-path/{group_path}/metadata-indexes", params(("group_path" = String, Path), SourceKeyQuery), responses((status = 200, body = [MetadataIndexResponse])))]
pub(crate) async fn list_metadata_indexes(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(group_path): Path<String>,
    Query(query): Query<SourceKeyQuery>,
) -> impl IntoResponse {
    let group = match group_for_user(&state, session.user.id, &group_path).await {
        Ok(value) => value,
        Err(error) => return group_access_error_response(error),
    };
    match state
        .app
        .document_store
        .list_indexes(group.id, &query.source_key)
        .await
    {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(post, path = "/v1/groups/by-path/{group_path}/metadata-indexes", params(("group_path" = String, Path), SourceKeyQuery), request_body = CreateMetadataIndexRequest, responses((status = 201, body = MetadataIndexResponse)))]
pub(crate) async fn create_metadata_index(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(group_path): Path<String>,
    Query(query): Query<SourceKeyQuery>,
    Json(request): Json<CreateMetadataIndexRequest>,
) -> impl IntoResponse {
    let group = match group_for_user(&state, session.user.id, &group_path).await {
        Ok(value) => value,
        Err(error) => return group_access_error_response(error),
    };
    if let Err(error) = require_group_role(&group, MembershipRole::Maintainer) {
        return group_access_error_response(error);
    }
    match state
        .app
        .document_store
        .create_index(group.id, &group_path, &query.source_key, &request)
        .await
    {
        Ok(value) => (StatusCode::CREATED, Json(value)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

async fn require_manager(
    state: &ApiState,
    user_id: i64,
    group_path: &str,
) -> Result<crate::domain::GroupRecord, axum::response::Response> {
    let group = group_for_user(state, user_id, group_path)
        .await
        .map_err(group_access_error_response)?;
    require_group_role(&group, MembershipRole::Maintainer).map_err(group_access_error_response)?;
    Ok(group)
}

#[utoipa::path(put, path = "/v1/groups/by-path/{group_path}/metadata-indexes/{index_id}", params(("group_path" = String, Path), ("index_id" = Uuid, Path)), request_body = UpdateMetadataIndexRequest, responses((status = 200, body = MetadataIndexResponse)))]
pub(crate) async fn update_metadata_index(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_path, index_id)): Path<(String, Uuid)>,
    Json(request): Json<UpdateMetadataIndexRequest>,
) -> impl IntoResponse {
    let group = match require_manager(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(response) => return response,
    };
    match state
        .app
        .document_store
        .update_index(group.id, index_id, &request)
        .await
    {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(post, path = "/v1/groups/by-path/{group_path}/metadata-indexes/{index_id}/retry", params(("group_path" = String, Path), ("index_id" = Uuid, Path)), responses((status = 200, body = MetadataIndexResponse)))]
pub(crate) async fn retry_metadata_index(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_path, index_id)): Path<(String, Uuid)>,
) -> impl IntoResponse {
    let group = match require_manager(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(response) => return response,
    };
    match state
        .app
        .document_store
        .retry_index(group.id, index_id)
        .await
    {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(delete, path = "/v1/groups/by-path/{group_path}/metadata-indexes/{index_id}", params(("group_path" = String, Path), ("index_id" = Uuid, Path)), responses((status = 204)))]
pub(crate) async fn delete_metadata_index(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_path, index_id)): Path<(String, Uuid)>,
) -> impl IntoResponse {
    let group = match require_manager(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(response) => return response,
    };
    match state
        .app
        .document_store
        .delete_index(group.id, index_id)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => library_management_error_response(error),
    }
}
