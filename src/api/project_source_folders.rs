use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use uuid::Uuid;

use crate::contracts::{
    CreateSourceFolderRequest, MembershipRole, SourceConfigInput, SourceFolderResponse, SyncOutcome,
};

use super::{
    ApiState,
    auth::CurrentUser,
    errors::library_management_error_response,
    project_access::{project_access_error_response, project_for_user, require_project_role},
};

#[utoipa::path(
    post,
    path = "/v1/groups/{group_key}/projects/{project_key}/source-folders",
    params(
        ("group_key" = String, Path, description = "Group key"),
        ("project_key" = String, Path, description = "Project key")
    ),
    request_body = CreateSourceFolderRequest,
    responses(
        (status = 201, description = "Created source folder", body = SourceFolderResponse),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Project not found")
    )
)]
pub(crate) async fn create_project_source_folder(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_key, project_key)): Path<(String, String)>,
    Json(request): Json<CreateSourceFolderRequest>,
) -> impl IntoResponse {
    let project = match project_for_user(&state, session.user.id, &group_key, &project_key).await {
        Ok(project) => project,
        Err(error) => return project_access_error_response(error),
    };
    if let Err(error) = require_project_role(&project, MembershipRole::Maintainer) {
        return project_access_error_response(error);
    }
    match state
        .app
        .source_folders
        .create_source_folder_in_project(&project, &request)
        .await
    {
        Ok(response) => (StatusCode::CREATED, Json(response)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(
    put,
    path = "/v1/groups/{group_key}/projects/{project_key}/source-folders/{folder_id}/config",
    params(
        ("group_key" = String, Path, description = "Group key"),
        ("project_key" = String, Path, description = "Project key"),
        ("folder_id" = Uuid, Path, description = "Source folder id")
    ),
    request_body = SourceConfigInput,
    responses(
        (status = 200, description = "Updated source folder config", body = SourceFolderResponse),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Project or source folder not found")
    )
)]
pub(crate) async fn update_project_source_folder_config(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_key, project_key, folder_id)): Path<(String, String, Uuid)>,
    Json(request): Json<SourceConfigInput>,
) -> impl IntoResponse {
    let project = match project_for_user(&state, session.user.id, &group_key, &project_key).await {
        Ok(project) => project,
        Err(error) => return project_access_error_response(error),
    };
    if let Err(error) = require_project_role(&project, MembershipRole::Maintainer) {
        return project_access_error_response(error);
    }
    match state
        .app
        .source_folders
        .update_source_folder_config_in_project(&project, folder_id, &request)
        .await
    {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/v1/groups/{group_key}/projects/{project_key}/source-folders/{folder_id}/sync",
    params(
        ("group_key" = String, Path, description = "Group key"),
        ("project_key" = String, Path, description = "Project key"),
        ("folder_id" = Uuid, Path, description = "Source folder id")
    ),
    responses(
        (status = 202, description = "Triggered source folder sync", body = SyncOutcome),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Project or source folder not found")
    )
)]
pub(crate) async fn sync_project_source_folder(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_key, project_key, folder_id)): Path<(String, String, Uuid)>,
) -> impl IntoResponse {
    let project = match project_for_user(&state, session.user.id, &group_key, &project_key).await {
        Ok(project) => project,
        Err(error) => return project_access_error_response(error),
    };
    if let Err(error) = require_project_role(&project, MembershipRole::Maintainer) {
        return project_access_error_response(error);
    }
    match state
        .app
        .source_folders
        .sync_source_folder_in_project(&project, folder_id)
        .await
    {
        Ok(response) => (StatusCode::ACCEPTED, Json(response)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}
