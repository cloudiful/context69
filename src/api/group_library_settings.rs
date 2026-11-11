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
    ApiErrorResponse, GroupLibrarySettingsResponse, MembershipRole,
    UpdateGroupLibrarySettingsRequest,
};

#[utoipa::path(get, path = "/v1/groups/by-path/{group_path}/library-settings", params(("group_path" = String, Path)), responses((status = 200, body = GroupLibrarySettingsResponse), (status = 404, body = ApiErrorResponse)))]
pub(crate) async fn get_group_library_settings(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(group_path): Path<String>,
) -> impl IntoResponse {
    let group = match group_for_user(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    match state.app.library.group_library_settings(group.id).await {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(put, path = "/v1/groups/by-path/{group_path}/library-settings", params(("group_path" = String, Path)), request_body = UpdateGroupLibrarySettingsRequest, responses((status = 200, body = GroupLibrarySettingsResponse), (status = 403, body = ApiErrorResponse), (status = 404, body = ApiErrorResponse)))]
pub(crate) async fn update_group_library_settings(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(group_path): Path<String>,
    Json(request): Json<UpdateGroupLibrarySettingsRequest>,
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
        .library
        .update_group_library_settings(group.id, &request)
        .await
    {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}
