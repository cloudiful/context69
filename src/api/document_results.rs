use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};

use super::{
    ApiState,
    auth::CurrentUser,
    errors::library_management_error_response,
    group_access::{group_access_error_response, group_for_user, require_group_role},
};
use crate::contracts::{CreateDocumentResultRequest, ListDocumentResultsQuery, MembershipRole};

#[utoipa::path(post, path = "/v1/groups/by-path/{group_path}/documents/{document_id}/results", params(("group_path" = String, Path), ("document_id" = i64, Path)),
    request_body = CreateDocumentResultRequest,
    responses((status = 200, description = "Document result appended to the caller's results", body = crate::contracts::DocumentResultResponse), (status = 400, description = "Invalid result payload", body = crate::contracts::ApiErrorResponse), (status = 403, description = "Insufficient group role", body = crate::contracts::ApiErrorResponse), (status = 404, description = "Document or group not found", body = crate::contracts::ApiErrorResponse), (status = 409, description = "Document result conflict", body = crate::contracts::ApiErrorResponse)))]
pub(crate) async fn create_document_result(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_path, document_id)): Path<(String, i64)>,
    Json(request): Json<CreateDocumentResultRequest>,
) -> impl IntoResponse {
    let group = match group_for_user(&state, &session, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    if let Err(error) = require_group_role(&state, &session, &group, MembershipRole::Maintainer) {
        return group_access_error_response(error);
    }
    if let Err(error) = request.validate() {
        return library_management_error_response(
            crate::domain_errors::DomainError::invalid_argument(error.to_string()).into(),
        );
    }
    let scope = match state
        .app
        .auth
        .access_scope(
            Some(session.user.id),
            Some(group_path.clone()),
            session.token_group_scope.as_ref(),
        )
        .await
    {
        Ok(scope) => scope,
        Err(error) => return group_access_error_response(error),
    };
    match state
        .app
        .db
        .append_document_result(
            group.id,
            document_id,
            &group_path,
            session.user.id,
            &scope,
            &request,
        )
        .await
    {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(get, path = "/v1/groups/by-path/{group_path}/documents/{document_id}/results", params(("group_path" = String, Path), ("document_id" = i64, Path), ListDocumentResultsQuery),
    responses((status = 200, description = "Page of document results for the caller", body = crate::contracts::ListDocumentResultsResponse), (status = 400, description = "Invalid result list query", body = crate::contracts::ApiErrorResponse), (status = 404, description = "Document or group not found", body = crate::contracts::ApiErrorResponse)))]
pub(crate) async fn list_document_results(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_path, document_id)): Path<(String, i64)>,
    Query(query): Query<ListDocumentResultsQuery>,
) -> impl IntoResponse {
    let group = match group_for_user(&state, &session, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    let scope = match state
        .app
        .auth
        .access_scope(
            Some(session.user.id),
            Some(group_path.clone()),
            session.token_group_scope.as_ref(),
        )
        .await
    {
        Ok(scope) => scope,
        Err(error) => return group_access_error_response(error),
    };
    match state
        .app
        .db
        .list_document_results(group.id, document_id, &group_path, &scope, &query)
        .await
    {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}
