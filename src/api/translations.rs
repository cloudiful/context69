use axum::{
    Json,
    extract::{Path, State},
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
    ApiErrorResponse, GroupTranslationSettingsResponse, MembershipRole,
    RebuildDocumentTranslationsRequest, TranslationJobResponse, TranslationJobsResponse,
    TranslationSettingsResponse, UpdateGroupTranslationSettingsRequest,
    UpdateTranslationSettingsRequest,
};

#[utoipa::path(get, path = "/v1/settings/translation", responses((status = 200, body = TranslationSettingsResponse), (status = 403, body = ApiErrorResponse)))]
pub(crate) async fn get_translation_settings(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
) -> impl IntoResponse {
    if !session.user.is_admin {
        return admin_required();
    }
    match state.app.translation.settings().await {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(put, path = "/v1/settings/translation", request_body = UpdateTranslationSettingsRequest, responses((status = 200, body = TranslationSettingsResponse), (status = 403, body = ApiErrorResponse)))]
pub(crate) async fn update_translation_settings(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Json(request): Json<UpdateTranslationSettingsRequest>,
) -> impl IntoResponse {
    if !session.user.is_admin {
        return admin_required();
    }
    match state.app.translation.update_settings(&request).await {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

fn admin_required() -> axum::response::Response {
    (
        StatusCode::FORBIDDEN,
        Json(ApiErrorResponse {
            error: "administrator access required".to_string(),
        }),
    )
        .into_response()
}

#[utoipa::path(get, path = "/v1/groups/by-path/{group_path}/translation-settings", params(("group_path" = String, Path)), responses((status = 200, body = GroupTranslationSettingsResponse)))]
pub(crate) async fn get_group_translation_settings(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(group_path): Path<String>,
) -> impl IntoResponse {
    let group = match group_for_user(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    match state.app.translation.group_settings(group.id).await {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(put, path = "/v1/groups/by-path/{group_path}/translation-settings", params(("group_path" = String, Path)), request_body = UpdateGroupTranslationSettingsRequest, responses((status = 200, body = GroupTranslationSettingsResponse)))]
pub(crate) async fn update_group_translation_settings(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(group_path): Path<String>,
    Json(request): Json<UpdateGroupTranslationSettingsRequest>,
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
        .translation
        .update_group_settings(group.id, &request)
        .await
    {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(get, path = "/v1/groups/by-path/{group_path}/documents/{document_id}/translations", params(("group_path" = String, Path), ("document_id" = i64, Path)), responses((status = 200, body = TranslationJobsResponse)))]
pub(crate) async fn list_document_translation_jobs(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_path, document_id)): Path<(String, i64)>,
) -> impl IntoResponse {
    let group = match group_for_user(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    match state
        .app
        .translation
        .jobs_for_document(group.id, document_id)
        .await
    {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(post, path = "/v1/groups/by-path/{group_path}/documents/{document_id}/translations/rebuild", params(("group_path" = String, Path), ("document_id" = i64, Path)), request_body = RebuildDocumentTranslationsRequest, responses((status = 202, body = TranslationJobsResponse)))]
pub(crate) async fn rebuild_document_translations(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_path, document_id)): Path<(String, i64)>,
    Json(request): Json<RebuildDocumentTranslationsRequest>,
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
        .translation
        .rebuild_document(group.id, document_id, &request)
        .await
    {
        Ok(value) => (StatusCode::ACCEPTED, Json(value)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(get, path = "/v1/groups/by-path/{group_path}/translation-jobs/{job_id}", params(("group_path" = String, Path), ("job_id" = Uuid, Path)), responses((status = 200, body = TranslationJobResponse)))]
pub(crate) async fn get_translation_job(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_path, job_id)): Path<(String, Uuid)>,
) -> impl IntoResponse {
    let group = match group_for_user(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    match state.app.translation.job(group.id, job_id).await {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(post, path = "/v1/groups/by-path/{group_path}/translation-jobs/{job_id}/retry", params(("group_path" = String, Path), ("job_id" = Uuid, Path)), responses((status = 202, body = TranslationJobResponse)))]
pub(crate) async fn retry_translation_job(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_path, job_id)): Path<(String, Uuid)>,
) -> impl IntoResponse {
    let group = match group_for_user(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    if let Err(error) = require_group_role(&group, MembershipRole::Maintainer) {
        return group_access_error_response(error);
    }
    match state.app.translation.retry_job(group.id, job_id).await {
        Ok(value) => (StatusCode::ACCEPTED, Json(value)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}
