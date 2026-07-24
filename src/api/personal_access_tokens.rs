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
        PersonalAccessTokenPageQuery, PersonalAccessTokenPageResponse, PersonalAccessTokenResponse,
    },
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
            &request.name,
            &request.scopes,
            request.expires_in_days,
        )
        .await
    {
        Ok(created) => {
            (StatusCode::OK, Json(create_response_from_created(created))).into_response()
        }
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response(),
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
        Err(error) if error.to_string().contains("not found") => (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response(),
        Err(error) => internal_error_response(error),
    }
}

fn response_from_view(view: PersonalAccessTokenView) -> PersonalAccessTokenResponse {
    PersonalAccessTokenResponse {
        token_id: view.token_id,
        name: view.name,
        display_prefix: view.display_prefix,
        scopes: view.scopes,
        expires_at: view.expires_at,
        last_used_at: view.last_used_at,
        revoked_at: view.revoked_at,
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
