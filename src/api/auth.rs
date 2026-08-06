use std::collections::BTreeSet;

use axum::{
    Json,
    extract::{FromRequestParts, State},
    http::{StatusCode, header, request::Parts},
    middleware::Next,
    response::{IntoResponse, Response},
};
use axum_login::AuthSession as AxumAuthSession;
use context69_http_support::AuthenticatedUser;
use uuid::Uuid;

use crate::{
    contracts::{ApiErrorResponse, AuthLoginRequest, AuthMeResponse, PersonalAccessTokenScope},
    services::auth::{AuthService, AuthSession, Credentials, user_response},
    services::personal_access_tokens::is_personal_access_token,
};

use super::{ApiState, errors::internal_error_response};

#[derive(Clone)]
pub(crate) enum AuthKind {
    BrowserSession,
    PersonalAccessToken {
        token_id: Uuid,
        scopes: BTreeSet<PersonalAccessTokenScope>,
    },
}

pub(crate) type BrowserAuthSession = AxumAuthSession<AuthService>;

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
                Json(ApiErrorResponse::new(
                    "unauthorized",
                    "missing authenticated session or personal access token".to_string(),
                )),
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
    let bearer = extract_bearer_token(request.headers()).map(|token| token.map(str::to_owned));
    let browser_session = request
        .extensions()
        .get::<BrowserAuthSession>()
        .and_then(|auth| auth.user.as_ref())
        .map(|principal| principal.0.clone());
    match authenticate_request(&state, bearer, browser_session).await {
        Ok(Some(authenticated)) => {
            request
                .extensions_mut()
                .insert(authenticated_user(&authenticated.session));
            request
                .extensions_mut()
                .insert(RequestAuth(Some(authenticated)));
            next.run(request).await
        }
        Ok(None) => (
            StatusCode::UNAUTHORIZED,
            Json(ApiErrorResponse::new(
                "unauthorized",
                "missing authenticated session or personal access token".to_string(),
            )),
        )
            .into_response(),
        Err(error) => (
            StatusCode::UNAUTHORIZED,
            Json(ApiErrorResponse::new("unauthorized", error)),
        )
            .into_response(),
    }
}

pub(crate) async fn optional_auth_middleware(
    State(state): State<ApiState>,
    mut request: axum::http::Request<axum::body::Body>,
    next: Next,
) -> Response {
    let bearer = extract_bearer_token(request.headers()).map(|token| token.map(str::to_owned));
    let browser_session = request
        .extensions()
        .get::<BrowserAuthSession>()
        .and_then(|auth| auth.user.as_ref())
        .map(|principal| principal.0.clone());
    match authenticate_request(&state, bearer, browser_session).await {
        Ok(Some(authenticated)) => {
            request
                .extensions_mut()
                .insert(authenticated_user(&authenticated.session));
            request
                .extensions_mut()
                .insert(RequestAuth(Some(authenticated)));
            next.run(request).await
        }
        Ok(None) => {
            request.extensions_mut().insert(RequestAuth(None));
            next.run(request).await
        }
        Err(error) => (
            StatusCode::UNAUTHORIZED,
            Json(ApiErrorResponse::new("unauthorized", error)),
        )
            .into_response(),
    }
}

fn authenticated_user(session: &AuthSession) -> AuthenticatedUser {
    AuthenticatedUser {
        user_id: session.user.id,
        login_name: session.user.login_name.clone(),
        display_name: session.user.display_name.clone(),
        is_admin: session.user.is_admin,
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
            Json(ApiErrorResponse::new(
                "forbidden",
                "personal access tokens cannot manage personal access tokens".to_string(),
            )),
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
        && let Err(error) = state
            .app
            .personal_access_tokens
            .touch_last_used(*token_id)
            .await
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
        (status = 204, description = "Authenticated session"),
        (status = 401, description = "Invalid login or password", body = ApiErrorResponse)
    )
)]
pub(crate) async fn login(
    mut auth_session: BrowserAuthSession,
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
pub(crate) async fn logout(mut auth_session: BrowserAuthSession) -> impl IntoResponse {
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
    bearer: Result<Option<String>, String>,
    browser_session: Option<AuthSession>,
) -> Result<Option<AuthenticatedRequest>, String> {
    if let Some(token) = bearer? {
        if !is_personal_access_token(&token) {
            return Err("bearer token must be a personal access token".to_string());
        }
        let verified = state
            .app
            .personal_access_tokens
            .verify(&token)
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
    Ok(browser_session.map(|session| AuthenticatedRequest {
        session,
        kind: AuthKind::BrowserSession,
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
            Json(ApiErrorResponse::new(
                "unauthorized",
                "missing authenticated session or personal access token".to_string(),
            )),
        )
            .into_response();
    };

    if let AuthKind::PersonalAccessToken { token_id, scopes } = authenticated.kind {
        if !scopes.contains(&required_scope) {
            return (
                StatusCode::FORBIDDEN,
                Json(ApiErrorResponse::new(
                    "forbidden",
                    format!(
                        "personal access token missing {} scope",
                        scope_name(required_scope)
                    ),
                )),
            )
                .into_response();
        }

        if let Err(error) = state
            .app
            .personal_access_tokens
            .touch_last_used(token_id)
            .await
        {
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

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, HeaderValue, header};

    use super::extract_bearer_token;

    #[test]
    fn bearer_extraction_preserves_pat_and_rejects_other_schemes() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer ctx_pat_secret"),
        );
        assert_eq!(extract_bearer_token(&headers), Ok(Some("ctx_pat_secret")));

        headers.insert(header::AUTHORIZATION, HeaderValue::from_static("Basic abc"));
        assert_eq!(
            extract_bearer_token(&headers),
            Err("authorization header must use Bearer".to_string())
        );
    }
}
