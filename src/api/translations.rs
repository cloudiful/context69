use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use context69_contracts::TaskKind;
use serde_json::json;

use super::{
    ApiState,
    auth::CurrentUser,
    errors::library_management_error_response,
    group_access::{group_access_error_response, group_for_user, require_group_role},
};
use crate::contracts::{
    ApiErrorResponse, GroupTranslationSettingsResponse, MembershipRole,
    RebuildDocumentTranslationsRequest, TaskRef, TranslationJobsResponse,
    TranslationProviderPageQuery, TranslationProviderPageResponse, TranslationSettingsResponse,
    UpdateGroupTranslationSettingsRequest, UpdateTranslationSettingsRequest,
};
use crate::services::tasks::TaskSubmission;

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

#[utoipa::path(get, path = "/v1/settings/translation/providers", params(TranslationProviderPageQuery), responses((status = 200, body = TranslationProviderPageResponse), (status = 403, body = ApiErrorResponse)))]
pub(crate) async fn list_translation_providers(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Query(query): Query<TranslationProviderPageQuery>,
) -> impl IntoResponse {
    if !session.user.is_admin {
        return admin_required();
    }
    match state
        .app
        .translation
        .provider_page(query.page, query.page_size)
        .await
    {
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
        Json(ApiErrorResponse::new(
            "forbidden",
            "administrator access required".to_string(),
        )),
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

#[utoipa::path(post, path = "/v1/groups/by-path/{group_path}/documents/{document_id}/translations/rebuild", params(("group_path" = String, Path), ("document_id" = i64, Path)), request_body = RebuildDocumentTranslationsRequest, responses((status = 202, body = TaskRef), (status = 403), (status = 404)))]
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
        .tasks
        .submit(TaskSubmission {
            user_id: session.user.id,
            group_id: Some(group.id),
            group_path: Some(group_path),
            source_key: None,
            kind: TaskKind::Translation,
            payloads: vec![json!({
                "document_id": document_id,
                "target_locales": request.target_locales,
            })],
            input_storage_object_ids: Vec::new(),
            idempotency_key: None,
        })
        .await
    {
        Ok(value) => (StatusCode::ACCEPTED, Json(value)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}
