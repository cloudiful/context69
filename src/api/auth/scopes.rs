use axum::{
    Json,
    extract::State,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::{
    api::errors::internal_error_response,
    contracts::{ApiErrorResponse, PersonalAccessTokenScope},
};

use super::{ApiState, AuthKind, RequestAuth};

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
