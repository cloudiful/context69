use axum::{Json, http::StatusCode, response::IntoResponse};

use crate::{
    api::errors::internal_error_response,
    contracts::{ApiErrorResponse, AuthLoginRequest, AuthMeResponse},
    services::auth::{Credentials, user_response},
};

use super::{BrowserAuthSession, CurrentUser};

#[utoipa::path(
    post,
    path = "/v1/auth/login",
    request_body = AuthLoginRequest,
    responses(
        (status = 204, description = "Authenticated session"),
        (status = 401, description = "Invalid login or password", body = ApiErrorResponse)
    )
)]
pub(crate) async fn login(
    auth_session: BrowserAuthSession,
    Json(request): Json<AuthLoginRequest>,
) -> impl IntoResponse {
    let credentials = Credentials {
        login_name: request.login_name,
        password: request.password,
    };
    match auth_session.authenticate(credentials).await {
        Ok(Some(principal)) => match auth_session.login(&principal).await {
            Ok(()) => StatusCode::NO_CONTENT.into_response(),
            Err(error) => internal_error_response(anyhow::anyhow!(error)),
        },
        Ok(None) => (
            StatusCode::UNAUTHORIZED,
            Json(ApiErrorResponse::new(
                "unauthorized",
                "invalid login or password".to_string(),
            )),
        )
            .into_response(),
        Err(error) => internal_error_response(anyhow::anyhow!(error)),
    }
}

#[utoipa::path(
    post,
    path = "/v1/auth/logout",
    responses(
        (status = 204, description = "Logged out"),
        (status = 500, description = "Internal error", body = ApiErrorResponse)
    )
)]
pub(crate) async fn logout(auth_session: BrowserAuthSession) -> impl IntoResponse {
    match auth_session.logout().await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => internal_error_response(anyhow::anyhow!(error)),
    }
}

#[utoipa::path(
    get,
    path = "/v1/auth/me",
    responses(
        (status = 200, description = "Current authenticated user", body = crate::contracts::AuthMeResponse),
        (status = 401, description = "Missing or invalid session", body = ApiErrorResponse)
    )
)]
pub(crate) async fn me(CurrentUser(session): CurrentUser) -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(AuthMeResponse {
            user: user_response(&session),
        }),
    )
        .into_response()
}
