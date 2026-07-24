use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};

use crate::{
    contracts::{
        AdminUserPageQuery, AdminUserPageResponse, AdminUserResponse, ApiErrorResponse,
        CreateAdminUserRequest, ResetAdminUserPasswordRequest, UpdateAdminUserRequest,
    },
    domain::UserRecord,
};

use super::{ApiState, auth::CurrentUser, errors::admin_user_error_response};

#[utoipa::path(
    get,
    path = "/v1/admin/users",
    params(AdminUserPageQuery),
    responses(
        (status = 200, description = "Paginated users", body = AdminUserPageResponse),
        (status = 403, description = "Admin access required", body = ApiErrorResponse)
    )
)]
pub(crate) async fn list_admin_users(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Query(query): Query<AdminUserPageQuery>,
) -> impl IntoResponse {
    match state
        .app
        .auth
        .list_admin_users(
            &session.user,
            query.page,
            query.page_size,
            query.query.as_deref().unwrap_or_default(),
        )
        .await
    {
        Ok(page) => (
            StatusCode::OK,
            Json(AdminUserPageResponse {
                items: page.users.into_iter().map(admin_user_response).collect(),
                pagination: page.pagination,
            }),
        )
            .into_response(),
        Err(error) => admin_user_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/v1/admin/users",
    request_body = CreateAdminUserRequest,
    responses(
        (status = 201, description = "Created user", body = AdminUserResponse),
        (status = 400, description = "Invalid user payload", body = ApiErrorResponse),
        (status = 403, description = "Admin access required", body = ApiErrorResponse)
    )
)]
pub(crate) async fn create_admin_user(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Json(request): Json<CreateAdminUserRequest>,
) -> impl IntoResponse {
    match state
        .app
        .auth
        .create_admin_user(
            &session.user,
            &request.login_name,
            &request.display_name,
            &request.password,
            request.is_admin,
        )
        .await
    {
        Ok(user) => (StatusCode::CREATED, Json(admin_user_response(user))).into_response(),
        Err(error) => admin_user_error_response(error),
    }
}

#[utoipa::path(
    patch,
    path = "/v1/admin/users/{login_name}",
    params(("login_name" = String, Path, description = "User login name")),
    request_body = UpdateAdminUserRequest,
    responses(
        (status = 200, description = "Updated user", body = AdminUserResponse),
        (status = 400, description = "Invalid update payload", body = ApiErrorResponse),
        (status = 403, description = "Admin access required", body = ApiErrorResponse),
        (status = 404, description = "User not found", body = ApiErrorResponse)
    )
)]
pub(crate) async fn update_admin_user(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(login_name): Path<String>,
    Json(request): Json<UpdateAdminUserRequest>,
) -> impl IntoResponse {
    match state
        .app
        .auth
        .update_admin_user(
            &session.user,
            &login_name,
            request.display_name.as_deref(),
            request.is_admin,
        )
        .await
    {
        Ok(user) => (StatusCode::OK, Json(admin_user_response(user))).into_response(),
        Err(error) => admin_user_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/v1/admin/users/{login_name}/reset-password",
    params(("login_name" = String, Path, description = "User login name")),
    request_body = ResetAdminUserPasswordRequest,
    responses(
        (status = 200, description = "Password reset", body = AdminUserResponse),
        (status = 400, description = "Invalid password payload", body = ApiErrorResponse),
        (status = 403, description = "Admin access required", body = ApiErrorResponse),
        (status = 404, description = "User not found", body = ApiErrorResponse)
    )
)]
pub(crate) async fn reset_admin_user_password(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(login_name): Path<String>,
    Json(request): Json<ResetAdminUserPasswordRequest>,
) -> impl IntoResponse {
    match state
        .app
        .auth
        .reset_admin_user_password(&session.user, &login_name, &request.password)
        .await
    {
        Ok(user) => (StatusCode::OK, Json(admin_user_response(user))).into_response(),
        Err(error) => admin_user_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/v1/admin/users/{login_name}/disable",
    params(("login_name" = String, Path, description = "User login name")),
    responses(
        (status = 200, description = "Disabled user", body = AdminUserResponse),
        (status = 400, description = "Invalid disable request", body = ApiErrorResponse),
        (status = 403, description = "Admin access required", body = ApiErrorResponse),
        (status = 404, description = "User not found", body = ApiErrorResponse)
    )
)]
pub(crate) async fn disable_admin_user(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(login_name): Path<String>,
) -> impl IntoResponse {
    match state
        .app
        .auth
        .disable_admin_user(&session.user, &login_name)
        .await
    {
        Ok(user) => (StatusCode::OK, Json(admin_user_response(user))).into_response(),
        Err(error) => admin_user_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/v1/admin/users/{login_name}/enable",
    params(("login_name" = String, Path, description = "User login name")),
    responses(
        (status = 200, description = "Enabled user", body = AdminUserResponse),
        (status = 403, description = "Admin access required", body = ApiErrorResponse),
        (status = 404, description = "User not found", body = ApiErrorResponse)
    )
)]
pub(crate) async fn enable_admin_user(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(login_name): Path<String>,
) -> impl IntoResponse {
    match state
        .app
        .auth
        .enable_admin_user(&session.user, &login_name)
        .await
    {
        Ok(user) => (StatusCode::OK, Json(admin_user_response(user))).into_response(),
        Err(error) => admin_user_error_response(error),
    }
}

fn admin_user_response(user: UserRecord) -> AdminUserResponse {
    AdminUserResponse {
        user_id: user.id,
        login_name: user.login_name,
        display_name: user.display_name,
        is_admin: user.is_admin,
        disabled_at: user.disabled_at,
        created_at: user.created_at,
        updated_at: user.updated_at,
    }
}
