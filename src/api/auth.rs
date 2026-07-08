use std::collections::BTreeSet;

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
use uuid::Uuid;

use crate::{
    contracts::{ApiErrorResponse, AuthLoginRequest, AuthMeResponse, PersonalAccessTokenScope},
    services::auth::{AuthSession, user_response},
    services::personal_access_tokens::is_personal_access_token,
};

use super::{ApiState, errors::internal_error_response};

#[derive(Clone)]
pub(crate) enum AuthKind {
    SessionJwt,
    PersonalAccessToken {
        token_id: Uuid,
        scopes: BTreeSet<PersonalAccessTokenScope>,
    },
}

#[derive(Clone)]
pub(crate) struct AuthenticatedRequest {
    pub session: AuthSession,
    pub kind: AuthKind,
}

#[derive(Clone)]
pub(crate) struct RequestAuth(pub Option<AuthenticatedRequest>);

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
            Some(authenticated) => Ok(Self(authenticated.session)),
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
    match authenticate_request(&state, request.headers()).await {
        Ok(Some(authenticated)) => {
            request.extensions_mut().insert(RequestAuth(Some(authenticated)));
            next.run(request).await
        }
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
    match authenticate_request(&state, request.headers()).await {
        Ok(Some(authenticated)) => {
            request.extensions_mut().insert(RequestAuth(Some(authenticated)));
            next.run(request).await
        }
        Ok(None) => {
            request.extensions_mut().insert(RequestAuth(None));
            next.run(request).await
        }
        Err(error) => (StatusCode::UNAUTHORIZED, Json(ApiErrorResponse { error })).into_response(),
    }
}

pub(crate) async fn forbid_personal_access_token_middleware(
    State(_state): State<ApiState>,
    request: axum::http::Request<axum::body::Body>,
    next: Next,
) -> Response {
    if let Some(RequestAuth(Some(AuthenticatedRequest {
        kind: AuthKind::PersonalAccessToken { .. },
        ..
    }))) = request.extensions().get::<RequestAuth>()
    {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiErrorResponse {
                error: "personal access tokens cannot manage personal access tokens".to_string(),
            }),
        )
            .into_response();
    }

    next.run(request).await
}

pub(crate) async fn touch_personal_access_token_middleware(
    State(state): State<ApiState>,
    request: axum::http::Request<axum::body::Body>,
    next: Next,
) -> Response {
    if let Some(RequestAuth(Some(AuthenticatedRequest {
        kind: AuthKind::PersonalAccessToken { token_id, .. },
        ..
    }))) = request.extensions().get::<RequestAuth>()
        && let Err(error) = state.app.personal_access_tokens.touch_last_used(*token_id).await
    {
        return internal_error_response(error);
    }

    next.run(request).await
}

pub(crate) async fn require_search_scope_middleware(
    State(state): State<ApiState>,
    request: axum::http::Request<axum::body::Body>,
    next: Next,
) -> Response {
    require_scope_middleware(state, request, next, PersonalAccessTokenScope::Search).await
}

pub(crate) async fn require_workspace_scope_middleware(
    State(state): State<ApiState>,
    request: axum::http::Request<axum::body::Body>,
    next: Next,
) -> Response {
    require_scope_middleware(state, request, next, PersonalAccessTokenScope::Workspace).await
}

pub(crate) async fn require_library_scope_middleware(
    State(state): State<ApiState>,
    request: axum::http::Request<axum::body::Body>,
    next: Next,
) -> Response {
    require_scope_middleware(state, request, next, PersonalAccessTokenScope::Library).await
}

pub(crate) async fn require_sources_scope_middleware(
    State(state): State<ApiState>,
    request: axum::http::Request<axum::body::Body>,
    next: Next,
) -> Response {
    require_scope_middleware(state, request, next, PersonalAccessTokenScope::Sources).await
}

pub(crate) async fn require_settings_scope_middleware(
    State(state): State<ApiState>,
    request: axum::http::Request<axum::body::Body>,
    next: Next,
) -> Response {
    require_scope_middleware(state, request, next, PersonalAccessTokenScope::Settings).await
}

pub(crate) async fn require_admin_scope_middleware(
    State(state): State<ApiState>,
    request: axum::http::Request<axum::body::Body>,
    next: Next,
) -> Response {
    require_scope_middleware(state, request, next, PersonalAccessTokenScope::Admin).await
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

async fn authenticate_request(
    state: &ApiState,
    headers: &axum::http::HeaderMap,
) -> Result<Option<AuthenticatedRequest>, String> {
    let Some(token) = extract_bearer_token(headers)? else {
        return Ok(None);
    };

    if is_personal_access_token(token) {
        let verified = state
            .app
            .personal_access_tokens
            .verify(token)
            .await
            .map_err(|error| error.to_string())?;
        return Ok(Some(AuthenticatedRequest {
            session: verified.session,
            kind: AuthKind::PersonalAccessToken {
                token_id: verified.token_id,
                scopes: verified.scopes,
            },
        }));
    }

    let session = state
        .app
        .auth
        .verify_access_token(token)
        .await
        .map_err(|error| error.to_string())?;
    Ok(Some(AuthenticatedRequest {
        session,
        kind: AuthKind::SessionJwt,
    }))
}

async fn require_scope_middleware(
    state: ApiState,
    request: axum::http::Request<axum::body::Body>,
    next: Next,
    required_scope: PersonalAccessTokenScope,
) -> Response {
    let auth = request.extensions().get::<RequestAuth>().cloned();
    let Some(RequestAuth(Some(authenticated))) = auth else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(ApiErrorResponse {
                error: "missing bearer token".to_string(),
            }),
        )
            .into_response();
    };

    if let AuthKind::PersonalAccessToken { token_id, scopes } = authenticated.kind {
        if !scopes.contains(&required_scope) {
            return (
                StatusCode::FORBIDDEN,
                Json(ApiErrorResponse {
                    error: format!("personal access token missing {} scope", scope_name(required_scope)),
                }),
            )
                .into_response();
        }

        if let Err(error) = state.app.personal_access_tokens.touch_last_used(token_id).await {
            return internal_error_response(error);
        }
    }

    next.run(request).await
}

fn scope_name(scope: PersonalAccessTokenScope) -> &'static str {
    match scope {
        PersonalAccessTokenScope::Search => "search",
        PersonalAccessTokenScope::Workspace => "workspace",
        PersonalAccessTokenScope::Library => "library",
        PersonalAccessTokenScope::Sources => "sources",
        PersonalAccessTokenScope::Settings => "settings",
        PersonalAccessTokenScope::Admin => "admin",
    }
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
