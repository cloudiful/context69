use axum::{
    Json,
    extract::State,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};

use uuid::Uuid;

use crate::contracts::{ApiErrorResponse, PersonalAccessTokenScope};

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

    match admit_personal_access_token(&authenticated.kind, required_scope) {
        ScopeAdmission::Allowed(token_id) => {
            state.app.personal_access_tokens.record_last_used(token_id);
        }
        ScopeAdmission::MissingScope => {
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
        ScopeAdmission::NotAPersonalAccessToken => {}
    }

    next.run(request).await
}

/// What a scope-gated request observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScopeAdmission {
    /// The request carried a personal access token with the required scope.
    Allowed(Uuid),
    /// The request carried a personal access token without the required scope.
    MissingScope,
    /// The request carried no personal access token.
    NotAPersonalAccessToken,
}

/// The admission a scope-gated request earns.
///
/// The usage observation is admitted only with the scope, so a refused scope
/// and a browser session record nothing.
fn admit_personal_access_token(
    kind: &AuthKind,
    required_scope: PersonalAccessTokenScope,
) -> ScopeAdmission {
    match kind {
        AuthKind::PersonalAccessToken { token_id, scopes } if scopes.contains(&required_scope) => {
            ScopeAdmission::Allowed(*token_id)
        }
        AuthKind::PersonalAccessToken { .. } => ScopeAdmission::MissingScope,
        AuthKind::BrowserSession => ScopeAdmission::NotAPersonalAccessToken,
    }
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
    use std::collections::BTreeSet;

    use uuid::Uuid;

    use super::{AuthKind, ScopeAdmission, admit_personal_access_token};
    use crate::contracts::PersonalAccessTokenScope;

    #[test]
    fn a_scope_gated_request_observes_usage_only_after_its_scope_passed() {
        let token_id = Uuid::new_v4();
        let permitted = AuthKind::PersonalAccessToken {
            token_id,
            scopes: BTreeSet::from([PersonalAccessTokenScope::Search]),
        };
        assert_eq!(
            admit_personal_access_token(&permitted, PersonalAccessTokenScope::Search),
            ScopeAdmission::Allowed(token_id)
        );
        assert_eq!(
            admit_personal_access_token(&permitted, PersonalAccessTokenScope::Admin),
            ScopeAdmission::MissingScope,
            "a refused scope observes no usage"
        );

        assert_eq!(
            admit_personal_access_token(
                &AuthKind::BrowserSession,
                PersonalAccessTokenScope::Search
            ),
            ScopeAdmission::NotAPersonalAccessToken,
            "a browser session observes no usage"
        );
    }
}
