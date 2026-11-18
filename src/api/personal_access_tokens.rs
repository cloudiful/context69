use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use uuid::Uuid;

use crate::{
    contracts::{
        ApiErrorResponse, CreatePersonalAccessTokenRequest, CreatePersonalAccessTokenResponse,
        GroupPageResponse, NamespacePageQuery, PersonalAccessTokenPageQuery,
        PersonalAccessTokenPageResponse, PersonalAccessTokenResponse,
        RotatePersonalAccessTokenRequest, UpdatePersonalAccessTokenRequest,
    },
    http_adapters::{group_page_response, page_request},
    services::personal_access_tokens::{CreatedPersonalAccessToken, PersonalAccessTokenView},
};

use super::{ApiState, auth::CurrentUser, errors::internal_error_response};

#[utoipa::path(
    get,
    path = "/v1/auth/personal-access-tokens",
    params(PersonalAccessTokenPageQuery),
    responses(
        (status = 200, description = "Paginated personal access tokens for current user", body = PersonalAccessTokenPageResponse),
        (status = 401, description = "Missing or invalid bearer token", body = ApiErrorResponse),
        (status = 403, description = "Personal access tokens cannot manage personal access tokens", body = ApiErrorResponse)
    )
)]
pub(crate) async fn list_personal_access_tokens(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Query(query): Query<PersonalAccessTokenPageQuery>,
) -> impl IntoResponse {
    match state
        .app
        .personal_access_tokens
        .list_page_for_user(session.user.id, query.page, query.page_size)
        .await
    {
        Ok(page) => (
            StatusCode::OK,
            Json(PersonalAccessTokenPageResponse {
                items: page.items.into_iter().map(response_from_view).collect(),
                pagination: page.pagination,
            }),
        )
            .into_response(),
        Err(error) => internal_error_response(error),
    }
}

#[utoipa::path(
    get,
    path = "/v1/auth/personal-access-tokens/group-catalog",
    params(NamespacePageQuery),
    responses(
        (status = 200, description = "Paged catalog of every group currently accessible to the token owner", body = GroupPageResponse),
        (status = 400, description = "Invalid page request", body = ApiErrorResponse),
        (status = 401, description = "Missing or invalid browser session", body = ApiErrorResponse),
        (status = 403, description = "Personal access tokens cannot manage personal access tokens", body = ApiErrorResponse)
    )
)]
pub(crate) async fn list_personal_access_token_group_catalog(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Query(query): Query<NamespacePageQuery>,
) -> impl IntoResponse {
    match state
        .app
        .namespace
        .list_accessible_groups_for_user(session.user.id, &page_request(&query))
        .await
    {
        Ok(page) => (StatusCode::OK, Json(group_page_response(page))).into_response(),
        Err(error) => internal_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/v1/auth/personal-access-tokens",
    request_body = CreatePersonalAccessTokenRequest,
    responses(
        (status = 200, description = "Create a new personal access token", body = CreatePersonalAccessTokenResponse),
        (status = 400, description = "Invalid personal access token request", body = ApiErrorResponse),
        (status = 401, description = "Missing or invalid bearer token", body = ApiErrorResponse),
        (status = 403, description = "Personal access tokens cannot manage personal access tokens", body = ApiErrorResponse)
    )
)]
pub(crate) async fn create_personal_access_token(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Json(request): Json<CreatePersonalAccessTokenRequest>,
) -> impl IntoResponse {
    match state
        .app
        .personal_access_tokens
        .create_for_user(
            session.user.id,
            session.user.is_admin,
            &request.name,
            &request.scopes,
            request.group_scope.as_ref(),
            request.expires_in_days,
        )
        .await
    {
        Ok(created) => {
            (StatusCode::OK, Json(create_response_from_created(created))).into_response()
        }
        Err(error) => context69_http_support::json_error_for_code(
            context69_contracts::ApiErrorCode::InvalidArgument,
            error.to_string(),
        ),
    }
}

#[utoipa::path(
    delete,
    path = "/v1/auth/personal-access-tokens/{token_id}",
    params(("token_id" = Uuid, Path, description = "Personal access token id")),
    responses(
        (status = 204, description = "Personal access token revoked"),
        (status = 401, description = "Missing or invalid bearer token", body = ApiErrorResponse),
        (status = 403, description = "Personal access tokens cannot manage personal access tokens", body = ApiErrorResponse),
        (status = 404, description = "Personal access token not found", body = ApiErrorResponse)
    )
)]
pub(crate) async fn revoke_personal_access_token(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(token_id): Path<Uuid>,
) -> impl IntoResponse {
    match state
        .app
        .personal_access_tokens
        .revoke_for_user(session.user.id, token_id)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => {
            if context69_http_support::is_not_found_error(&error) {
                context69_http_support::json_error_for_code(
                    context69_contracts::ApiErrorCode::NotFound,
                    error.to_string(),
                )
            } else {
                internal_error_response(error)
            }
        }
    }
}

#[utoipa::path(
    patch,
    path = "/v1/auth/personal-access-tokens/{token_id}",
    params(("token_id" = Uuid, Path, description = "Personal access token id")),
    request_body = UpdatePersonalAccessTokenRequest,
    responses(
        (status = 200, description = "Updated personal access token", body = PersonalAccessTokenResponse),
        (status = 400, description = "Invalid personal access token request", body = ApiErrorResponse),
        (status = 401, description = "Missing or invalid bearer token", body = ApiErrorResponse),
        (status = 403, description = "Personal access tokens cannot manage personal access tokens", body = ApiErrorResponse),
        (status = 404, description = "Personal access token not found", body = ApiErrorResponse),
        (status = 409, description = "Personal access token changed since it was read", body = ApiErrorResponse)
    )
)]
pub(crate) async fn update_personal_access_token(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(token_id): Path<Uuid>,
    Json(request): Json<UpdatePersonalAccessTokenRequest>,
) -> impl IntoResponse {
    match state
        .app
        .personal_access_tokens
        .update_for_user(
            session.user.id,
            session.user.is_admin,
            token_id,
            request.name.as_deref(),
            request.scopes.as_deref(),
            request.group_scope.as_ref(),
        )
        .await
    {
        Ok(view) => (StatusCode::OK, Json(response_from_view(view))).into_response(),
        Err(error) => personal_access_token_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/v1/auth/personal-access-tokens/{token_id}/rotate",
    params(("token_id" = Uuid, Path, description = "Personal access token id")),
    request_body = RotatePersonalAccessTokenRequest,
    responses(
        (status = 200, description = "Replacement personal access token", body = CreatePersonalAccessTokenResponse),
        (status = 400, description = "Invalid personal access token request", body = ApiErrorResponse),
        (status = 401, description = "Missing or invalid bearer token", body = ApiErrorResponse),
        (status = 403, description = "Personal access tokens cannot manage personal access tokens", body = ApiErrorResponse),
        (status = 404, description = "Personal access token not found", body = ApiErrorResponse),
        (status = 409, description = "Personal access token changed since it was read", body = ApiErrorResponse)
    )
)]
pub(crate) async fn rotate_personal_access_token(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(token_id): Path<Uuid>,
    Json(request): Json<RotatePersonalAccessTokenRequest>,
) -> impl IntoResponse {
    match state
        .app
        .personal_access_tokens
        .rotate_for_user(
            session.user.id,
            session.user.is_admin,
            token_id,
            request.expires_in_days,
        )
        .await
    {
        Ok(created) => {
            (StatusCode::OK, Json(create_response_from_created(created))).into_response()
        }
        Err(error) => personal_access_token_error_response(error),
    }
}

fn personal_access_token_error_response(error: anyhow::Error) -> axum::response::Response {
    if context69_http_support::is_not_found_error(&error) {
        context69_http_support::json_error_for_code(
            context69_contracts::ApiErrorCode::NotFound,
            error.to_string(),
        )
    } else if context69_http_support::is_invalid_argument_error(&error) {
        context69_http_support::json_error_for_code(
            context69_contracts::ApiErrorCode::InvalidArgument,
            error.to_string(),
        )
    } else if context69_http_support::find_domain_error(&error)
        .is_some_and(|error| matches!(error, crate::domain_errors::DomainError::Forbidden(_)))
    {
        context69_http_support::json_error_for_code(
            context69_contracts::ApiErrorCode::Forbidden,
            error.to_string(),
        )
    } else if context69_http_support::find_domain_error(&error)
        .is_some_and(|error| matches!(error, crate::domain_errors::DomainError::Conflict(_)))
    {
        context69_http_support::json_error_for_code(
            context69_contracts::ApiErrorCode::Conflict,
            error.to_string(),
        )
    } else {
        context69_http_support::internal_error_response(error)
    }
}

fn response_from_view(view: PersonalAccessTokenView) -> PersonalAccessTokenResponse {
    PersonalAccessTokenResponse {
        token_id: view.token_id,
        name: view.name,
        display_prefix: view.display_prefix,
        scopes: view.scopes,
        group_scope: view.group_scope,
        expires_at: view.expires_at,
        last_used_at: view.last_used_at,
        last_used_precision: view.last_used_precision,
        revoked_at: view.revoked_at,
        rotated_from_token_id: view.rotated_from_token_id,
        created_at: view.created_at,
        updated_at: view.updated_at,
    }
}

fn create_response_from_created(
    created: CreatedPersonalAccessToken,
) -> CreatePersonalAccessTokenResponse {
    CreatePersonalAccessTokenResponse {
        access_token: created.access_token,
        token: response_from_view(created.token),
    }
}
