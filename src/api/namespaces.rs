use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};

use crate::{
    contracts::{
        ApiErrorResponse, CreateGroupRequest, CreateProjectRequest, GroupMemberResponse,
        GroupResponse, MoveProjectRequest, ProjectMemberResponse, ProjectResponse,
        UpdateGroupRequest, UpdateProjectRequest, UpsertMembershipRequest,
    },
    domain::{GroupRecord, NamespaceMemberRecord, ProjectRecord},
};

use super::{ApiState, auth::CurrentUser, errors::internal_error_response};

#[utoipa::path(
    get,
    path = "/v1/groups",
    responses(
        (status = 200, description = "Visible groups", body = [GroupResponse]),
        (status = 401, description = "Missing or invalid bearer token", body = ApiErrorResponse)
    )
)]
pub(crate) async fn list_groups(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
) -> impl IntoResponse {
    match state.app.db.list_groups_for_user(session.user.id).await {
        Ok(groups) => (
            StatusCode::OK,
            Json(groups.into_iter().map(group_response).collect::<Vec<_>>()),
        )
            .into_response(),
        Err(error) => internal_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/v1/groups",
    request_body = CreateGroupRequest,
    responses(
        (status = 201, description = "Created group", body = GroupResponse),
        (status = 400, description = "Invalid group request", body = ApiErrorResponse),
        (status = 401, description = "Missing or invalid bearer token", body = ApiErrorResponse)
    )
)]
pub(crate) async fn create_group(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Json(request): Json<CreateGroupRequest>,
) -> impl IntoResponse {
    match state.app.db.create_group(&session.user, &request).await {
        Ok(group) => (StatusCode::CREATED, Json(group_response(group))).into_response(),
        Err(error) => namespace_error_response(error),
    }
}

#[utoipa::path(
    get,
    path = "/v1/groups/{group_key}",
    params(("group_key" = String, Path, description = "Group key")),
    responses(
        (status = 200, description = "Group details", body = GroupResponse),
        (status = 404, description = "Group not found", body = ApiErrorResponse)
    )
)]
pub(crate) async fn get_group(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(group_key): Path<String>,
) -> impl IntoResponse {
    match state
        .app
        .db
        .get_group_for_user(session.user.id, &group_key)
        .await
    {
        Ok(Some(group)) => (StatusCode::OK, Json(group_response(group))).into_response(),
        Ok(None) => not_found("group"),
        Err(error) => internal_error_response(error),
    }
}

#[utoipa::path(
    patch,
    path = "/v1/groups/{group_key}",
    params(("group_key" = String, Path, description = "Group key")),
    request_body = UpdateGroupRequest,
    responses(
        (status = 200, description = "Updated group", body = GroupResponse),
        (status = 404, description = "Group not found", body = ApiErrorResponse)
    )
)]
pub(crate) async fn update_group(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(group_key): Path<String>,
    Json(request): Json<UpdateGroupRequest>,
) -> impl IntoResponse {
    match state
        .app
        .db
        .update_group(&session.user, &group_key, &request)
        .await
    {
        Ok(group) => (StatusCode::OK, Json(group_response(group))).into_response(),
        Err(error) => namespace_error_response(error),
    }
}

#[utoipa::path(
    delete,
    path = "/v1/groups/{group_key}",
    params(("group_key" = String, Path, description = "Group key")),
    responses((status = 204, description = "Deleted group")))
]
pub(crate) async fn delete_group(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(group_key): Path<String>,
) -> impl IntoResponse {
    match state.app.db.delete_group(&session.user, &group_key).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => namespace_error_response(error),
    }
}

#[utoipa::path(
    get,
    path = "/v1/groups/{group_key}/members",
    params(("group_key" = String, Path, description = "Group key")),
    responses((status = 200, description = "Group members", body = [GroupMemberResponse])))
]
pub(crate) async fn list_group_members(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(group_key): Path<String>,
) -> impl IntoResponse {
    match state
        .app
        .db
        .list_group_members(&session.user, &group_key)
        .await
    {
        Ok(members) => (
            StatusCode::OK,
            Json(
                members
                    .into_iter()
                    .map(group_member_response)
                    .collect::<Vec<_>>(),
            ),
        )
            .into_response(),
        Err(error) => namespace_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/v1/groups/{group_key}/members",
    params(("group_key" = String, Path, description = "Group key")),
    request_body = UpsertMembershipRequest,
    responses((status = 204, description = "Saved group member")))
]
pub(crate) async fn upsert_group_member(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(group_key): Path<String>,
    Json(request): Json<UpsertMembershipRequest>,
) -> impl IntoResponse {
    match state
        .app
        .db
        .upsert_group_member(&session.user, &group_key, &request)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => namespace_error_response(error),
    }
}

#[utoipa::path(
    delete,
    path = "/v1/groups/{group_key}/members/{login_name}",
    params(
        ("group_key" = String, Path, description = "Group key"),
        ("login_name" = String, Path, description = "User login name")
    ),
    responses((status = 204, description = "Removed group member")))
]
pub(crate) async fn delete_group_member(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_key, login_name)): Path<(String, String)>,
) -> impl IntoResponse {
    match state
        .app
        .db
        .delete_group_member(&session.user, &group_key, &login_name)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => namespace_error_response(error),
    }
}

#[utoipa::path(
    get,
    path = "/v1/groups/{group_key}/projects",
    params(("group_key" = String, Path, description = "Group key")),
    responses((status = 200, description = "Visible projects", body = [ProjectResponse])))
]
pub(crate) async fn list_projects(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(group_key): Path<String>,
) -> impl IntoResponse {
    match state
        .app
        .db
        .list_projects_for_user_in_group(session.user.id, &group_key)
        .await
    {
        Ok(projects) => (
            StatusCode::OK,
            Json(
                projects
                    .into_iter()
                    .map(project_response)
                    .collect::<Vec<_>>(),
            ),
        )
            .into_response(),
        Err(error) => namespace_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/v1/groups/{group_key}/projects",
    params(("group_key" = String, Path, description = "Group key")),
    request_body = CreateProjectRequest,
    responses((status = 201, description = "Created project", body = ProjectResponse)))
]
pub(crate) async fn create_project(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(group_key): Path<String>,
    Json(request): Json<CreateProjectRequest>,
) -> impl IntoResponse {
    match state
        .app
        .db
        .create_project(&session.user, &group_key, &request)
        .await
    {
        Ok(project) => (StatusCode::CREATED, Json(project_response(project))).into_response(),
        Err(error) => namespace_error_response(error),
    }
}

#[utoipa::path(
    get,
    path = "/v1/groups/{group_key}/projects/{project_key}",
    params(
        ("group_key" = String, Path, description = "Group key"),
        ("project_key" = String, Path, description = "Project key")
    ),
    responses((status = 200, description = "Project details", body = ProjectResponse)))
]
pub(crate) async fn get_project(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_key, project_key)): Path<(String, String)>,
) -> impl IntoResponse {
    match state
        .app
        .db
        .get_project_for_user(session.user.id, &group_key, &project_key)
        .await
    {
        Ok(Some(project)) => (StatusCode::OK, Json(project_response(project))).into_response(),
        Ok(None) => not_found("project"),
        Err(error) => internal_error_response(error),
    }
}

#[utoipa::path(
    patch,
    path = "/v1/groups/{group_key}/projects/{project_key}",
    params(
        ("group_key" = String, Path, description = "Group key"),
        ("project_key" = String, Path, description = "Project key")
    ),
    request_body = UpdateProjectRequest,
    responses((status = 200, description = "Updated project", body = ProjectResponse)))
]
pub(crate) async fn update_project(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_key, project_key)): Path<(String, String)>,
    Json(request): Json<UpdateProjectRequest>,
) -> impl IntoResponse {
    match state
        .app
        .db
        .update_project(&session.user, &group_key, &project_key, &request)
        .await
    {
        Ok(project) => (StatusCode::OK, Json(project_response(project))).into_response(),
        Err(error) => namespace_error_response(error),
    }
}

#[utoipa::path(
    delete,
    path = "/v1/groups/{group_key}/projects/{project_key}",
    params(
        ("group_key" = String, Path, description = "Group key"),
        ("project_key" = String, Path, description = "Project key")
    ),
    responses((status = 204, description = "Deleted project")))
]
pub(crate) async fn delete_project(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_key, project_key)): Path<(String, String)>,
) -> impl IntoResponse {
    match state
        .app
        .db
        .delete_project(&session.user, &group_key, &project_key)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => namespace_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/v1/groups/{group_key}/projects/{project_key}/move",
    params(
        ("group_key" = String, Path, description = "Source group key"),
        ("project_key" = String, Path, description = "Project key")
    ),
    request_body = MoveProjectRequest,
    responses((status = 200, description = "Moved project", body = ProjectResponse))
)]
pub(crate) async fn move_project(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_key, project_key)): Path<(String, String)>,
    Json(request): Json<MoveProjectRequest>,
) -> impl IntoResponse {
    match state
        .app
        .db
        .move_project(&session.user, &group_key, &project_key, &request)
        .await
    {
        Ok(project) => (StatusCode::OK, Json(project_response(project))).into_response(),
        Err(error) => namespace_error_response(error),
    }
}

#[utoipa::path(
    get,
    path = "/v1/groups/{group_key}/projects/{project_key}/members",
    params(
        ("group_key" = String, Path, description = "Group key"),
        ("project_key" = String, Path, description = "Project key")
    ),
    responses((status = 200, description = "Project members", body = [ProjectMemberResponse])))
]
pub(crate) async fn list_project_members(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_key, project_key)): Path<(String, String)>,
) -> impl IntoResponse {
    match state
        .app
        .db
        .list_project_members(&session.user, &group_key, &project_key)
        .await
    {
        Ok(members) => (
            StatusCode::OK,
            Json(
                members
                    .into_iter()
                    .map(project_member_response)
                    .collect::<Vec<_>>(),
            ),
        )
            .into_response(),
        Err(error) => namespace_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/v1/groups/{group_key}/projects/{project_key}/members",
    params(
        ("group_key" = String, Path, description = "Group key"),
        ("project_key" = String, Path, description = "Project key")
    ),
    request_body = UpsertMembershipRequest,
    responses((status = 204, description = "Saved project member")))
]
pub(crate) async fn upsert_project_member(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_key, project_key)): Path<(String, String)>,
    Json(request): Json<UpsertMembershipRequest>,
) -> impl IntoResponse {
    match state
        .app
        .db
        .upsert_project_member(&session.user, &group_key, &project_key, &request)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => namespace_error_response(error),
    }
}

#[utoipa::path(
    delete,
    path = "/v1/groups/{group_key}/projects/{project_key}/members/{login_name}",
    params(
        ("group_key" = String, Path, description = "Group key"),
        ("project_key" = String, Path, description = "Project key"),
        ("login_name" = String, Path, description = "User login name")
    ),
    responses((status = 204, description = "Removed project member")))
]
pub(crate) async fn delete_project_member(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_key, project_key, login_name)): Path<(String, String, String)>,
) -> impl IntoResponse {
    match state
        .app
        .db
        .delete_project_member(&session.user, &group_key, &project_key, &login_name)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => namespace_error_response(error),
    }
}

fn namespace_error_response(error: anyhow::Error) -> axum::response::Response {
    let message = error.to_string();
    let status = if message.contains("unknown group")
        || message.contains("unknown project")
        || message.contains("unknown user")
    {
        StatusCode::NOT_FOUND
    } else if message.contains("insufficient permissions") {
        StatusCode::FORBIDDEN
    } else if message.contains("must not be empty")
        || message.contains("cannot be broader")
        || message.contains("only admins")
        || message.contains("personal groups")
        || message.contains("already exists")
        || message.contains("user account is disabled")
        || message.contains("invalid")
    {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };

    (status, Json(ApiErrorResponse { error: message })).into_response()
}

fn not_found(resource: &str) -> axum::response::Response {
    (
        StatusCode::NOT_FOUND,
        Json(ApiErrorResponse {
            error: format!("{resource} not found"),
        }),
    )
        .into_response()
}

fn group_response(group: GroupRecord) -> GroupResponse {
    GroupResponse {
        group_id: group.id,
        group_key: group.group_key,
        parent_group_key: group.parent_group_key,
        name: group.name,
        visibility: group.visibility,
        kind: group.kind,
        current_role: group.current_role,
        created_at: group.created_at,
        updated_at: group.updated_at,
    }
}

fn project_response(project: ProjectRecord) -> ProjectResponse {
    ProjectResponse {
        project_id: project.id,
        group_key: project.group_key,
        project_key: project.project_key,
        name: project.name,
        visibility: project.visibility,
        current_role: project.current_role,
        created_at: project.created_at,
        updated_at: project.updated_at,
    }
}

fn group_member_response(member: NamespaceMemberRecord) -> GroupMemberResponse {
    GroupMemberResponse {
        user_id: member.user_id,
        login_name: member.login_name,
        display_name: member.display_name,
        role: member.role,
    }
}

fn project_member_response(member: NamespaceMemberRecord) -> ProjectMemberResponse {
    ProjectMemberResponse {
        user_id: member.user_id,
        login_name: member.login_name,
        display_name: member.display_name,
        role: member.role,
    }
}

#[cfg(test)]
mod tests {
    use axum::{body, http::StatusCode};

    use super::namespace_error_response;

    #[tokio::test]
    async fn insufficient_permissions_maps_to_forbidden() {
        let response =
            namespace_error_response(anyhow::anyhow!("insufficient permissions for project"));
        assert_eq!(response.status(), StatusCode::FORBIDDEN);

        let bytes = body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body to read");
        let payload = String::from_utf8(bytes.to_vec()).expect("utf8 body");
        assert!(payload.contains("insufficient permissions for project"));
    }
}
