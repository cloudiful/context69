use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};

use super::{
    ApiState,
    auth::CurrentUser,
    errors::library_management_error_response,
    group_access::{group_access_error_response, group_for_user, require_group_role},
};
use crate::contracts::{
    ExtractionJobsResponse, ExtractionTemplateInput, ExtractionTemplateResponse, MembershipRole,
    RebuildDocumentExtractionsRequest,
};

#[utoipa::path(get, path = "/v1/groups/by-path/{group_path}/extraction-templates", params(("group_path" = String, Path)), responses((status = 200, body = Vec<ExtractionTemplateResponse>)))]
pub(crate) async fn list_extraction_templates(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(group_path): Path<String>,
) -> impl IntoResponse {
    let group = match group_for_user(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    if let Err(error) = require_group_role(&group, MembershipRole::Maintainer) {
        return group_access_error_response(error);
    }
    match state.app.extraction.templates().await {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(put, path = "/v1/groups/by-path/{group_path}/extraction-templates", params(("group_path" = String, Path)), request_body = ExtractionTemplateInput, responses((status = 200, body = ExtractionTemplateResponse)))]
pub(crate) async fn upsert_extraction_template(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(group_path): Path<String>,
    Json(request): Json<ExtractionTemplateInput>,
) -> impl IntoResponse {
    let group = match group_for_user(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    if let Err(error) = require_group_role(&group, MembershipRole::Maintainer) {
        return group_access_error_response(error);
    }
    match state.app.extraction.register_template(&request).await {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(get, path = "/v1/groups/by-path/{group_path}/documents/{document_id}/extractions", params(("group_path" = String, Path), ("document_id" = i64, Path)), responses((status = 200, body = ExtractionJobsResponse)))]
pub(crate) async fn list_document_extraction_jobs(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_path, document_id)): Path<(String, i64)>,
) -> impl IntoResponse {
    let group = match group_for_user(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    match state.app.extraction.jobs(group.id, document_id).await {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(post, path = "/v1/groups/by-path/{group_path}/documents/{document_id}/extractions/rebuild", params(("group_path" = String, Path), ("document_id" = i64, Path)), request_body = RebuildDocumentExtractionsRequest, responses((status = 200, body = ExtractionJobsResponse)))]
pub(crate) async fn rebuild_document_extractions(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_path, document_id)): Path<(String, i64)>,
    Json(request): Json<RebuildDocumentExtractionsRequest>,
) -> impl IntoResponse {
    let group = match group_for_user(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    if let Err(error) = require_group_role(&group, MembershipRole::Maintainer) {
        return group_access_error_response(error);
    }
    match state
        .app
        .extraction
        .rebuild(group.id, document_id, &request)
        .await
    {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}
