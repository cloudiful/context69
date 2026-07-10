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
    ApiErrorResponse, CreateGroupRequest, GroupMemberResponse, GroupResponse, MoveGroupRequest,
    UpdateGroupRequest, UpsertMembershipRequest, UserDirectoryEntryResponse,
};
use context69_http_support::{
    AuthenticatedUser, CurrentUser, internal_error_response, json_error_response,
};
use serde::Deserialize;
use utoipa::OpenApi;

#[async_trait]
pub trait NamespaceApi: Send + Sync {
    async fn list_groups_for_user(&self, user_id: i64) -> Result<Vec<GroupResponse>>;
    async fn list_child_groups_for_user(
        &self,
        user_id: i64,
        group_path: &str,
    ) -> Result<Vec<GroupResponse>>;
    async fn get_group_for_user(
        &self,
        user_id: i64,
        group_path: &str,
    ) -> Result<Option<GroupResponse>>;
    async fn create_group(
        &self,
        actor: &AuthenticatedUser,
        request: &CreateGroupRequest,
    ) -> Result<GroupResponse>;
    async fn update_group(
        &self,
        actor: &AuthenticatedUser,
        group_path: &str,
        request: &UpdateGroupRequest,
    ) -> Result<GroupResponse>;
    async fn move_group(
        &self,
        actor: &AuthenticatedUser,
        group_path: &str,
        request: &MoveGroupRequest,
    ) -> Result<GroupResponse>;
    async fn delete_group(&self, actor: &AuthenticatedUser, group_path: &str) -> Result<()>;
    async fn list_group_members(
        &self,
        actor: &AuthenticatedUser,
        group_path: &str,
    ) -> Result<Vec<GroupMemberResponse>>;
    async fn upsert_group_member(
        &self,
        actor: &AuthenticatedUser,
        group_path: &str,
        request: &UpsertMembershipRequest,
    ) -> Result<()>;
    async fn delete_group_member(
        &self,
        actor: &AuthenticatedUser,
        group_path: &str,
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
            "/v1/groups/by-path/{group_path}",
            get(get_group).patch(update_group).delete(delete_group),
        )
        .route("/v1/groups/by-path/{group_path}/move", post(move_group))
        .route(
            "/v1/groups/by-path/{group_path}/children",
            get(list_child_groups),
        )
        .route(
            "/v1/groups/by-path/{group_path}/members",
            get(list_group_members).post(upsert_group_member),
        )
        .route(
            "/v1/groups/by-path/{group_path}/members/{login_name}",
            delete(delete_group_member),
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
        move_group,
        delete_group,
        list_child_groups,
        list_group_members,
        upsert_group_member,
        delete_group_member
    ),
    components(
        schemas(
            ApiErrorResponse,
            UserDirectoryEntryResponse,
            context69_contracts::Visibility,
            context69_contracts::MembershipRole,
            context69_contracts::GroupKind,
            GroupResponse,
            GroupMemberResponse,
            CreateGroupRequest,
            UpdateGroupRequest,
            MoveGroupRequest,
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
        Err(error) => json_error_response(StatusCode::BAD_REQUEST, &error.to_string()),
    }
}

#[utoipa::path(get, path = "/v1/groups/by-path/{group_path}", params(("group_path" = String, Path)), responses((status = 200, body = GroupResponse), (status = 404, body = ApiErrorResponse)))]
async fn get_group(
    State(state): State<NamespaceHttpState>,
    CurrentUser(user): CurrentUser,
    Path(group_path): Path<String>,
) -> impl IntoResponse {
    match state
        .namespace
        .get_group_for_user(user.user_id, &group_path)
        .await
    {
        Ok(Some(group)) => (StatusCode::OK, axum::Json(group)).into_response(),
        Ok(None) => json_error_response(StatusCode::NOT_FOUND, "unknown group"),
        Err(error) => internal_error_response(error),
    }
}

#[utoipa::path(patch, path = "/v1/groups/by-path/{group_path}", params(("group_path" = String, Path)), request_body = UpdateGroupRequest, responses((status = 200, body = GroupResponse), (status = 404, body = ApiErrorResponse)))]
async fn update_group(
    State(state): State<NamespaceHttpState>,
    CurrentUser(user): CurrentUser,
    Path(group_path): Path<String>,
    axum::Json(request): axum::Json<UpdateGroupRequest>,
) -> impl IntoResponse {
    match state
        .namespace
        .update_group(&user, &group_path, &request)
        .await
    {
        Ok(group) => (StatusCode::OK, axum::Json(group)).into_response(),
        Err(error) => json_error_response(StatusCode::BAD_REQUEST, &error.to_string()),
    }
}

#[utoipa::path(post, path = "/v1/groups/by-path/{group_path}/move", params(("group_path" = String, Path)), request_body = MoveGroupRequest, responses((status = 200, body = GroupResponse)))]
async fn move_group(
    State(state): State<NamespaceHttpState>,
    CurrentUser(user): CurrentUser,
    Path(group_path): Path<String>,
    axum::Json(request): axum::Json<MoveGroupRequest>,
) -> impl IntoResponse {
    match state.namespace.move_group(&user, &group_path, &request).await {
        Ok(group) => (StatusCode::OK, axum::Json(group)).into_response(),
        Err(error) => json_error_response(StatusCode::BAD_REQUEST, &error.to_string()),
    }
}

#[utoipa::path(delete, path = "/v1/groups/by-path/{group_path}", params(("group_path" = String, Path)), responses((status = 204)))]
async fn delete_group(
    State(state): State<NamespaceHttpState>,
    CurrentUser(user): CurrentUser,
    Path(group_path): Path<String>,
) -> impl IntoResponse {
    match state.namespace.delete_group(&user, &group_path).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => json_error_response(StatusCode::BAD_REQUEST, &error.to_string()),
    }
}

#[utoipa::path(get, path = "/v1/groups/by-path/{group_path}/children", params(("group_path" = String, Path)), responses((status = 200, body = [GroupResponse])))]
async fn list_child_groups(
    State(state): State<NamespaceHttpState>,
    CurrentUser(user): CurrentUser,
    Path(group_path): Path<String>,
) -> impl IntoResponse {
    match state
        .namespace
        .list_child_groups_for_user(user.user_id, &group_path)
        .await
    {
        Ok(groups) => (StatusCode::OK, axum::Json(groups)).into_response(),
        Err(error) => internal_error_response(error),
    }
}

#[utoipa::path(get, path = "/v1/groups/by-path/{group_path}/members", params(("group_path" = String, Path)), responses((status = 200, body = [GroupMemberResponse])))]
async fn list_group_members(
    State(state): State<NamespaceHttpState>,
    CurrentUser(user): CurrentUser,
    Path(group_path): Path<String>,
) -> impl IntoResponse {
    match state.namespace.list_group_members(&user, &group_path).await {
        Ok(members) => (StatusCode::OK, axum::Json(members)).into_response(),
        Err(error) => json_error_response(StatusCode::BAD_REQUEST, &error.to_string()),
    }
}

#[utoipa::path(post, path = "/v1/groups/by-path/{group_path}/members", params(("group_path" = String, Path)), request_body = UpsertMembershipRequest, responses((status = 204)))]
async fn upsert_group_member(
    State(state): State<NamespaceHttpState>,
    CurrentUser(user): CurrentUser,
    Path(group_path): Path<String>,
    axum::Json(request): axum::Json<UpsertMembershipRequest>,
) -> impl IntoResponse {
    match state
        .namespace
        .upsert_group_member(&user, &group_path, &request)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => json_error_response(StatusCode::BAD_REQUEST, &error.to_string()),
    }
}

#[utoipa::path(delete, path = "/v1/groups/by-path/{group_path}/members/{login_name}", params(("group_path" = String, Path), ("login_name" = String, Path)), responses((status = 204)))]
async fn delete_group_member(
    State(state): State<NamespaceHttpState>,
    CurrentUser(user): CurrentUser,
    Path((group_path, login_name)): Path<(String, String)>,
) -> impl IntoResponse {
    match state
        .namespace
        .delete_group_member(&user, &group_path, &login_name)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => json_error_response(StatusCode::BAD_REQUEST, &error.to_string()),
    }
}
