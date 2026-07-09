use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use axum::{
    Router,
    extract::{FromRef, Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
};
use context69_contracts::{
    ApiErrorResponse, CreateGroupRequest, CreateProjectRequest, GroupMemberResponse, GroupResponse,
    MoveProjectRequest, ProjectMemberResponse, ProjectResponse, UpdateGroupRequest,
    UpdateProjectRequest, UpsertMembershipRequest, UserDirectoryEntryResponse,
};
use context69_http_support::{
    AuthenticatedUser, CurrentUser, internal_error_response, json_error_response,
};
use serde::Deserialize;
use utoipa::OpenApi;

#[async_trait]
pub trait NamespaceApi: Send + Sync {
    async fn list_groups_for_user(&self, user_id: i64) -> Result<Vec<GroupResponse>>;
    async fn get_group_for_user(
        &self,
        user_id: i64,
        group_key: &str,
    ) -> Result<Option<GroupResponse>>;
    async fn create_group(
        &self,
        actor: &AuthenticatedUser,
        request: &CreateGroupRequest,
    ) -> Result<GroupResponse>;
    async fn update_group(
        &self,
        actor: &AuthenticatedUser,
        group_key: &str,
        request: &UpdateGroupRequest,
    ) -> Result<GroupResponse>;
    async fn delete_group(&self, actor: &AuthenticatedUser, group_key: &str) -> Result<()>;
    async fn list_group_members(
        &self,
        actor: &AuthenticatedUser,
        group_key: &str,
    ) -> Result<Vec<GroupMemberResponse>>;
    async fn upsert_group_member(
        &self,
        actor: &AuthenticatedUser,
        group_key: &str,
        request: &UpsertMembershipRequest,
    ) -> Result<()>;
    async fn delete_group_member(
        &self,
        actor: &AuthenticatedUser,
        group_key: &str,
        login_name: &str,
    ) -> Result<()>;
    async fn list_projects_for_user_in_group(
        &self,
        user_id: i64,
        group_key: &str,
    ) -> Result<Vec<ProjectResponse>>;
    async fn get_project_for_user(
        &self,
        user_id: i64,
        group_key: &str,
        project_key: &str,
    ) -> Result<Option<ProjectResponse>>;
    async fn create_project(
        &self,
        actor: &AuthenticatedUser,
        group_key: &str,
        request: &CreateProjectRequest,
    ) -> Result<ProjectResponse>;
    async fn update_project(
        &self,
        actor: &AuthenticatedUser,
        group_key: &str,
        project_key: &str,
        request: &UpdateProjectRequest,
    ) -> Result<ProjectResponse>;
    async fn delete_project(
        &self,
        actor: &AuthenticatedUser,
        group_key: &str,
        project_key: &str,
    ) -> Result<()>;
    async fn move_project(
        &self,
        actor: &AuthenticatedUser,
        group_key: &str,
        project_key: &str,
        request: &MoveProjectRequest,
    ) -> Result<ProjectResponse>;
    async fn list_project_members(
        &self,
        actor: &AuthenticatedUser,
        group_key: &str,
        project_key: &str,
    ) -> Result<Vec<ProjectMemberResponse>>;
    async fn upsert_project_member(
        &self,
        actor: &AuthenticatedUser,
        group_key: &str,
        project_key: &str,
        request: &UpsertMembershipRequest,
    ) -> Result<()>;
    async fn delete_project_member(
        &self,
        actor: &AuthenticatedUser,
        group_key: &str,
        project_key: &str,
        login_name: &str,
    ) -> Result<()>;
}

#[async_trait]
pub trait UserDirectoryApi: Send + Sync {
    async fn search_user_directory(
        &self,
        actor: &AuthenticatedUser,
        query: &str,
        limit: usize,
    ) -> Result<Vec<UserDirectoryEntryResponse>>;
}

#[derive(Clone)]
pub struct NamespaceHttpState {
    pub namespace: Arc<dyn NamespaceApi>,
    pub user_directory: Arc<dyn UserDirectoryApi>,
}

pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    NamespaceHttpState: FromRef<S>,
{
    Router::new()
        .route("/v1/user-directory", get(search_user_directory))
        .route("/v1/groups", get(list_groups).post(create_group))
        .route(
            "/v1/groups/{group_key}",
            get(get_group).patch(update_group).delete(delete_group),
        )
        .route(
            "/v1/groups/{group_key}/members",
            get(list_group_members).post(upsert_group_member),
        )
        .route(
            "/v1/groups/{group_key}/members/{login_name}",
            delete(delete_group_member),
        )
        .route(
            "/v1/groups/{group_key}/projects",
            get(list_projects).post(create_project),
        )
        .route(
            "/v1/groups/{group_key}/projects/{project_key}",
            get(get_project)
                .patch(update_project)
                .delete(delete_project),
        )
        .route(
            "/v1/groups/{group_key}/projects/{project_key}/move",
            post(move_project),
        )
        .route(
            "/v1/groups/{group_key}/projects/{project_key}/members",
            get(list_project_members).post(upsert_project_member),
        )
        .route(
            "/v1/groups/{group_key}/projects/{project_key}/members/{login_name}",
            delete(delete_project_member),
        )
}

#[derive(OpenApi)]
#[openapi(
    paths(
        search_user_directory,
        list_groups,
        create_group,
        get_group,
        update_group,
        delete_group,
        list_group_members,
        upsert_group_member,
        delete_group_member,
        list_projects,
        create_project,
        get_project,
        update_project,
        delete_project,
        move_project,
        list_project_members,
        upsert_project_member,
        delete_project_member
    ),
    components(
        schemas(
            ApiErrorResponse,
            UserDirectoryEntryResponse,
            context69_contracts::Visibility,
            context69_contracts::MembershipRole,
            context69_contracts::GroupKind,
            GroupResponse,
            ProjectResponse,
            GroupMemberResponse,
            ProjectMemberResponse,
            CreateGroupRequest,
            UpdateGroupRequest,
            CreateProjectRequest,
            UpdateProjectRequest,
            MoveProjectRequest,
            UpsertMembershipRequest
        )
    ),
    tags((name = "workspace", description = "Workspace namespace transport"))
)]
struct NamespaceApiDoc;

pub fn openapi_document() -> utoipa::openapi::OpenApi {
    NamespaceApiDoc::openapi()
}

#[derive(Debug, Deserialize)]
struct UserDirectoryQuery {
    #[serde(default)]
    query: String,
    #[serde(default = "default_limit")]
    limit: usize,
}

const fn default_limit() -> usize {
    10
}

#[utoipa::path(
    get,
    path = "/v1/user-directory",
    params(
        ("query" = Option<String>, Query, description = "Search login_name or display_name"),
        ("limit" = Option<usize>, Query, description = "Max entries to return")
    ),
    responses(
        (status = 200, description = "Matching active users", body = [UserDirectoryEntryResponse]),
        (status = 401, description = "Missing or invalid bearer token", body = ApiErrorResponse)
    )
)]
async fn search_user_directory(
    State(state): State<NamespaceHttpState>,
    CurrentUser(user): CurrentUser,
    Query(query): Query<UserDirectoryQuery>,
) -> impl IntoResponse {
    match state
        .user_directory
        .search_user_directory(&user, &query.query, query.limit)
        .await
    {
        Ok(users) => (StatusCode::OK, axum::Json(users)).into_response(),
        Err(error) => internal_error_response(error),
    }
}

#[utoipa::path(get, path = "/v1/groups", responses((status = 200, body = [GroupResponse])))]
async fn list_groups(
    State(state): State<NamespaceHttpState>,
    CurrentUser(user): CurrentUser,
) -> impl IntoResponse {
    match state.namespace.list_groups_for_user(user.user_id).await {
        Ok(groups) => (StatusCode::OK, axum::Json(groups)).into_response(),
        Err(error) => internal_error_response(error),
    }
}

#[utoipa::path(post, path = "/v1/groups", request_body = CreateGroupRequest, responses((status = 201, body = GroupResponse), (status = 400, body = ApiErrorResponse)))]
async fn create_group(
    State(state): State<NamespaceHttpState>,
    CurrentUser(user): CurrentUser,
    axum::Json(request): axum::Json<CreateGroupRequest>,
) -> impl IntoResponse {
    match state.namespace.create_group(&user, &request).await {
        Ok(group) => (StatusCode::CREATED, axum::Json(group)).into_response(),
        Err(error) => namespace_error_response(error),
    }
}

#[utoipa::path(get, path = "/v1/groups/{group_key}", params(("group_key" = String, Path)), responses((status = 200, body = GroupResponse), (status = 404, body = ApiErrorResponse)))]
async fn get_group(
    State(state): State<NamespaceHttpState>,
    CurrentUser(user): CurrentUser,
    Path(group_key): Path<String>,
) -> impl IntoResponse {
    match state
        .namespace
        .get_group_for_user(user.user_id, &group_key)
        .await
    {
        Ok(Some(group)) => (StatusCode::OK, axum::Json(group)).into_response(),
        Ok(None) => not_found("group"),
        Err(error) => internal_error_response(error),
    }
}

#[utoipa::path(patch, path = "/v1/groups/{group_key}", params(("group_key" = String, Path)), request_body = UpdateGroupRequest, responses((status = 200, body = GroupResponse), (status = 404, body = ApiErrorResponse)))]
async fn update_group(
    State(state): State<NamespaceHttpState>,
    CurrentUser(user): CurrentUser,
    Path(group_key): Path<String>,
    axum::Json(request): axum::Json<UpdateGroupRequest>,
) -> impl IntoResponse {
    match state
        .namespace
        .update_group(&user, &group_key, &request)
        .await
    {
        Ok(group) => (StatusCode::OK, axum::Json(group)).into_response(),
        Err(error) => namespace_error_response(error),
    }
}

#[utoipa::path(delete, path = "/v1/groups/{group_key}", params(("group_key" = String, Path)), responses((status = 204)))]
async fn delete_group(
    State(state): State<NamespaceHttpState>,
    CurrentUser(user): CurrentUser,
    Path(group_key): Path<String>,
) -> impl IntoResponse {
    match state.namespace.delete_group(&user, &group_key).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => namespace_error_response(error),
    }
}

#[utoipa::path(get, path = "/v1/groups/{group_key}/members", params(("group_key" = String, Path)), responses((status = 200, body = [GroupMemberResponse])))]
async fn list_group_members(
    State(state): State<NamespaceHttpState>,
    CurrentUser(user): CurrentUser,
    Path(group_key): Path<String>,
) -> impl IntoResponse {
    match state.namespace.list_group_members(&user, &group_key).await {
        Ok(members) => (StatusCode::OK, axum::Json(members)).into_response(),
        Err(error) => namespace_error_response(error),
    }
}

#[utoipa::path(post, path = "/v1/groups/{group_key}/members", params(("group_key" = String, Path)), request_body = UpsertMembershipRequest, responses((status = 204)))]
async fn upsert_group_member(
    State(state): State<NamespaceHttpState>,
    CurrentUser(user): CurrentUser,
    Path(group_key): Path<String>,
    axum::Json(request): axum::Json<UpsertMembershipRequest>,
) -> impl IntoResponse {
    match state
        .namespace
        .upsert_group_member(&user, &group_key, &request)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => namespace_error_response(error),
    }
}

#[utoipa::path(delete, path = "/v1/groups/{group_key}/members/{login_name}", params(("group_key" = String, Path), ("login_name" = String, Path)), responses((status = 204)))]
async fn delete_group_member(
    State(state): State<NamespaceHttpState>,
    CurrentUser(user): CurrentUser,
    Path((group_key, login_name)): Path<(String, String)>,
) -> impl IntoResponse {
    match state
        .namespace
        .delete_group_member(&user, &group_key, &login_name)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => namespace_error_response(error),
    }
}

#[utoipa::path(get, path = "/v1/groups/{group_key}/projects", params(("group_key" = String, Path)), responses((status = 200, body = [ProjectResponse])))]
async fn list_projects(
    State(state): State<NamespaceHttpState>,
    CurrentUser(user): CurrentUser,
    Path(group_key): Path<String>,
) -> impl IntoResponse {
    match state
        .namespace
        .list_projects_for_user_in_group(user.user_id, &group_key)
        .await
    {
        Ok(projects) => (StatusCode::OK, axum::Json(projects)).into_response(),
        Err(error) => namespace_error_response(error),
    }
}

#[utoipa::path(post, path = "/v1/groups/{group_key}/projects", params(("group_key" = String, Path)), request_body = CreateProjectRequest, responses((status = 201, body = ProjectResponse)))]
async fn create_project(
    State(state): State<NamespaceHttpState>,
    CurrentUser(user): CurrentUser,
    Path(group_key): Path<String>,
    axum::Json(request): axum::Json<CreateProjectRequest>,
) -> impl IntoResponse {
    match state
        .namespace
        .create_project(&user, &group_key, &request)
        .await
    {
        Ok(project) => (StatusCode::CREATED, axum::Json(project)).into_response(),
        Err(error) => namespace_error_response(error),
    }
}

#[utoipa::path(get, path = "/v1/groups/{group_key}/projects/{project_key}", params(("group_key" = String, Path), ("project_key" = String, Path)), responses((status = 200, body = ProjectResponse), (status = 404, body = ApiErrorResponse)))]
async fn get_project(
    State(state): State<NamespaceHttpState>,
    CurrentUser(user): CurrentUser,
    Path((group_key, project_key)): Path<(String, String)>,
) -> impl IntoResponse {
    match state
        .namespace
        .get_project_for_user(user.user_id, &group_key, &project_key)
        .await
    {
        Ok(Some(project)) => (StatusCode::OK, axum::Json(project)).into_response(),
        Ok(None) => not_found("project"),
        Err(error) => internal_error_response(error),
    }
}

#[utoipa::path(patch, path = "/v1/groups/{group_key}/projects/{project_key}", params(("group_key" = String, Path), ("project_key" = String, Path)), request_body = UpdateProjectRequest, responses((status = 200, body = ProjectResponse)))]
async fn update_project(
    State(state): State<NamespaceHttpState>,
    CurrentUser(user): CurrentUser,
    Path((group_key, project_key)): Path<(String, String)>,
    axum::Json(request): axum::Json<UpdateProjectRequest>,
) -> impl IntoResponse {
    match state
        .namespace
        .update_project(&user, &group_key, &project_key, &request)
        .await
    {
        Ok(project) => (StatusCode::OK, axum::Json(project)).into_response(),
        Err(error) => namespace_error_response(error),
    }
}

#[utoipa::path(delete, path = "/v1/groups/{group_key}/projects/{project_key}", params(("group_key" = String, Path), ("project_key" = String, Path)), responses((status = 204)))]
async fn delete_project(
    State(state): State<NamespaceHttpState>,
    CurrentUser(user): CurrentUser,
    Path((group_key, project_key)): Path<(String, String)>,
) -> impl IntoResponse {
    match state
        .namespace
        .delete_project(&user, &group_key, &project_key)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => namespace_error_response(error),
    }
}

#[utoipa::path(post, path = "/v1/groups/{group_key}/projects/{project_key}/move", params(("group_key" = String, Path), ("project_key" = String, Path)), request_body = MoveProjectRequest, responses((status = 200, body = ProjectResponse)))]
async fn move_project(
    State(state): State<NamespaceHttpState>,
    CurrentUser(user): CurrentUser,
    Path((group_key, project_key)): Path<(String, String)>,
    axum::Json(request): axum::Json<MoveProjectRequest>,
) -> impl IntoResponse {
    match state
        .namespace
        .move_project(&user, &group_key, &project_key, &request)
        .await
    {
        Ok(project) => (StatusCode::OK, axum::Json(project)).into_response(),
        Err(error) => namespace_error_response(error),
    }
}

#[utoipa::path(get, path = "/v1/groups/{group_key}/projects/{project_key}/members", params(("group_key" = String, Path), ("project_key" = String, Path)), responses((status = 200, body = [ProjectMemberResponse])))]
async fn list_project_members(
    State(state): State<NamespaceHttpState>,
    CurrentUser(user): CurrentUser,
    Path((group_key, project_key)): Path<(String, String)>,
) -> impl IntoResponse {
    match state
        .namespace
        .list_project_members(&user, &group_key, &project_key)
        .await
    {
        Ok(members) => (StatusCode::OK, axum::Json(members)).into_response(),
        Err(error) => namespace_error_response(error),
    }
}

#[utoipa::path(post, path = "/v1/groups/{group_key}/projects/{project_key}/members", params(("group_key" = String, Path), ("project_key" = String, Path)), request_body = UpsertMembershipRequest, responses((status = 204)))]
async fn upsert_project_member(
    State(state): State<NamespaceHttpState>,
    CurrentUser(user): CurrentUser,
    Path((group_key, project_key)): Path<(String, String)>,
    axum::Json(request): axum::Json<UpsertMembershipRequest>,
) -> impl IntoResponse {
    match state
        .namespace
        .upsert_project_member(&user, &group_key, &project_key, &request)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => namespace_error_response(error),
    }
}

#[utoipa::path(delete, path = "/v1/groups/{group_key}/projects/{project_key}/members/{login_name}", params(("group_key" = String, Path), ("project_key" = String, Path), ("login_name" = String, Path)), responses((status = 204)))]
async fn delete_project_member(
    State(state): State<NamespaceHttpState>,
    CurrentUser(user): CurrentUser,
    Path((group_key, project_key, login_name)): Path<(String, String, String)>,
) -> impl IntoResponse {
    match state
        .namespace
        .delete_project_member(&user, &group_key, &project_key, &login_name)
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

    json_error_response(status, message)
}

fn not_found(resource: &str) -> axum::response::Response {
    json_error_response(StatusCode::NOT_FOUND, format!("{resource} not found"))
}
