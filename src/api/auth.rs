use axum::{
    Json,
    extract::{FromRequestParts, State},
    http::{StatusCode, header, request::Parts},
    middleware::Next,
    response::{IntoResponse, Response},
};
use axum_extra::extract::CookieJar;
use axum_extra::extract::cookie::{Cookie, SameSite};
use time::Duration as CookieDuration;

use crate::{
    contracts::{ApiErrorResponse, AuthLoginRequest, AuthMeResponse},
    services::auth::{AuthSession, user_response},
};

use super::{ApiState, errors::internal_error_response};

#[derive(Clone)]
pub(crate) struct RequestAuth(pub Option<AuthSession>);

#[derive(Clone)]
pub(crate) struct CurrentUser(pub AuthSession);

impl<S> FromRequestParts<S> for CurrentUser
where
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let auth = parts
            .extensions
            .get::<RequestAuth>()
            .cloned()
            .unwrap_or(RequestAuth(None));
        match auth.0 {
            Some(session) => Ok(Self(session)),
            None => Err((
                StatusCode::UNAUTHORIZED,
                Json(ApiErrorResponse {
                    error: "missing bearer token".to_string(),
                }),
            )
                .into_response()),
        }
    }
}

pub(crate) async fn auth_middleware(
    State(state): State<ApiState>,
    mut request: axum::http::Request<axum::body::Body>,
    next: Next,
) -> Response {
    match extract_bearer_token(request.headers()) {
        Ok(Some(token)) => match state.app.auth.verify_access_token(token).await {
            Ok(session) => {
                request.extensions_mut().insert(RequestAuth(Some(session)));
                next.run(request).await
            }
            Err(error) => (
                StatusCode::UNAUTHORIZED,
                Json(ApiErrorResponse {
                    error: error.to_string(),
                }),
            )
                .into_response(),
        },
        Ok(None) => (
            StatusCode::UNAUTHORIZED,
            Json(ApiErrorResponse {
                error: "missing bearer token".to_string(),
            }),
        )
            .into_response(),
        Err(error) => (StatusCode::UNAUTHORIZED, Json(ApiErrorResponse { error })).into_response(),
    }
}

pub(crate) async fn optional_auth_middleware(
    State(state): State<ApiState>,
    mut request: axum::http::Request<axum::body::Body>,
    next: Next,
) -> Response {
    match extract_bearer_token(request.headers()) {
        Ok(Some(token)) => match state.app.auth.verify_access_token(token).await {
            Ok(session) => {
                request.extensions_mut().insert(RequestAuth(Some(session)));
                next.run(request).await
            }
            Err(error) => (
                StatusCode::UNAUTHORIZED,
                Json(ApiErrorResponse {
                    error: error.to_string(),
                }),
            )
                .into_response(),
        },
        Ok(None) => {
            request.extensions_mut().insert(RequestAuth(None));
            next.run(request).await
        }
        Err(error) => (StatusCode::UNAUTHORIZED, Json(ApiErrorResponse { error })).into_response(),
    }
}

#[utoipa::path(
    post,
    path = "/v1/auth/login",
    request_body = AuthLoginRequest,
    responses(
        (status = 200, description = "Authenticated session", body = crate::contracts::AuthTokenResponse),
        (status = 401, description = "Invalid login or password", body = ApiErrorResponse)
    )
)]
pub(crate) async fn login(
    State(state): State<ApiState>,
    jar: CookieJar,
    Json(request): Json<AuthLoginRequest>,
) -> impl IntoResponse {
    match state
        .app
        .auth
        .login(&request.login_name, &request.password)
        .await
    {
        Ok(issued) => {
            let cookie = refresh_cookie(
                &state,
                state.app.auth.cookie_name(),
                &issued.refresh_token,
                state.app.auth.refresh_token_ttl_secs(),
            );
            let response = state.app.auth.token_response(issued);
            (jar.add(cookie), Json(response)).into_response()
        }
        Err(error) => (
            StatusCode::UNAUTHORIZED,
            Json(ApiErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response(),
    }
}

#[utoipa::path(
    post,
    path = "/v1/auth/refresh",
    responses(
        (status = 200, description = "Refreshed access token", body = crate::contracts::AuthTokenResponse),
        (status = 401, description = "Invalid refresh token", body = ApiErrorResponse)
    )
)]
pub(crate) async fn refresh(State(state): State<ApiState>, jar: CookieJar) -> impl IntoResponse {
    let Some(refresh_token) = jar
        .get(state.app.auth.cookie_name())
        .map(|cookie| cookie.value().to_string())
    else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(ApiErrorResponse {
                error: "missing refresh token".to_string(),
            }),
        )
            .into_response();
    };

    match state.app.auth.refresh(&refresh_token).await {
        Ok(issued) => {
            let cookie = refresh_cookie(
                &state,
                state.app.auth.cookie_name(),
                &issued.refresh_token,
                state.app.auth.refresh_token_ttl_secs(),
            );
            let response = state.app.auth.token_response(issued);
            (jar.add(cookie), Json(response)).into_response()
        }
        Err(error) => (
            StatusCode::UNAUTHORIZED,
            Json(ApiErrorResponse {
                error: error.to_string(),
            }),
        )
            .into_response(),
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
pub(crate) async fn logout(State(state): State<ApiState>, jar: CookieJar) -> impl IntoResponse {
    if let Some(refresh_token) = jar
        .get(state.app.auth.cookie_name())
        .map(|cookie| cookie.value().to_string())
        && let Err(error) = state.app.auth.logout(&refresh_token).await
    {
        return internal_error_response(error);
    }

    (
        jar.remove(clear_refresh_cookie(&state)),
        StatusCode::NO_CONTENT,
    )
        .into_response()
}

#[utoipa::path(
    get,
    path = "/v1/auth/me",
    responses(
        (status = 200, description = "Current authenticated user", body = crate::contracts::AuthMeResponse),
        (status = 401, description = "Missing or invalid bearer token", body = ApiErrorResponse)
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

pub(crate) fn extract_bearer_token(
    headers: &axum::http::HeaderMap,
) -> Result<Option<&str>, String> {
    let Some(value) = headers.get(header::AUTHORIZATION) else {
        return Ok(None);
    };
    let value = value
        .to_str()
        .map_err(|_| "invalid authorization header".to_string())?;
    let Some(token) = value.strip_prefix("Bearer ") else {
        return Err("authorization header must use Bearer".to_string());
    };
    let token = token.trim();
    if token.is_empty() {
        return Err("bearer token must not be empty".to_string());
    }
    Ok(Some(token))
}

fn refresh_cookie(state: &ApiState, name: &str, value: &str, ttl_secs: i64) -> Cookie<'static> {
    Cookie::build((name.to_string(), value.to_string()))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(state.app.auth.refresh_cookie_secure())
        .max_age(CookieDuration::seconds(ttl_secs))
        .build()
}

fn clear_refresh_cookie(state: &ApiState) -> Cookie<'static> {
    let mut cookie = Cookie::build((state.app.auth.cookie_name().to_string(), String::new()))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(state.app.auth.refresh_cookie_secure())
        .build();
    cookie.make_removal();
    cookie
}
