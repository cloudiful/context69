use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use context69_contracts::TaskKind;
use serde_json::json;
use uuid::Uuid;

use crate::contracts::{
    CreateSourceFolderRequest, MembershipRole, SourceConfigInput, SourceFolderResponse,
};

use super::{
    ApiState,
    auth::CurrentUser,
    errors::library_management_error_response,
    group_access::{group_access_error_response, group_for_user, require_group_role},
    submit_task_request,
};
use crate::services::tasks::TaskSubmission;

#[utoipa::path(
    post,
    path = "/v1/groups/by-path/{group_path}/source-folders",
    params(("group_path" = String, Path, description = "URL-encoded group path")),
    request_body = CreateSourceFolderRequest,
    responses(
        (status = 201, description = "Created source folder", body = SourceFolderResponse),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Group not found")
    )
)]
pub(crate) async fn create_group_source_folder(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(group_path): Path<String>,
    Json(request): Json<CreateSourceFolderRequest>,
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
        .source_folders
        .create_source_folder_in_project(&group, &request)
        .await
    {
        Ok(response) => (StatusCode::CREATED, Json(response)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(
    put,
    path = "/v1/groups/by-path/{group_path}/source-folders/{folder_id}/config",
    params(
        ("group_path" = String, Path, description = "URL-encoded group path"),
        ("folder_id" = Uuid, Path, description = "Source folder id")
    ),
    request_body = SourceConfigInput,
    responses(
        (status = 200, description = "Updated source folder config", body = SourceFolderResponse),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Group or source folder not found")
    )
)]
pub(crate) async fn update_group_source_folder_config(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_path, folder_id)): Path<(String, Uuid)>,
    Json(request): Json<SourceConfigInput>,
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
        .source_folders
        .update_source_folder_config_in_project(&group, folder_id, &request)
        .await
    {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/v1/groups/by-path/{group_path}/source-folders/{folder_id}/sync",
    params(
        ("group_path" = String, Path, description = "URL-encoded group path"),
        ("folder_id" = Uuid, Path, description = "Source folder id")
    ),
    responses(
        (status = 202, description = "Source folder sync task accepted", body = crate::contracts::TaskRef),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Group or source folder not found")
    )
)]
pub(crate) async fn sync_group_source_folder(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_path, folder_id)): Path<(String, Uuid)>,
) -> impl IntoResponse {
    let group = match group_for_user(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    if let Err(error) = require_group_role(&group, MembershipRole::Maintainer) {
        return group_access_error_response(error);
    }
    submit_task_request(
        &state,
        TaskSubmission {
            user_id: session.user.id,
            group_id: Some(group.id),
            group_path: Some(group.group_path),
            source_key: None,
            kind: TaskKind::SourceSync,
            payloads: vec![json!({"source_folder_id": folder_id})],
            idempotency_key: None,
        },
    )
    .await
}
