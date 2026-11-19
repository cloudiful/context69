use axum::{
    Json,
    extract::State,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::contracts::{ApiErrorResponse, PersonalAccessTokenScope};

use super::{ApiState, AuthKind, RequestAuth, records_successful_use};

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

    let token_id = match &authenticated.kind {
        AuthKind::PersonalAccessToken { token_id, .. } => {
            authenticated.telemetry_due.then_some(*token_id)
        }
        AuthKind::BrowserSession => None,
    };

    // The shared engine is the single scope authority: it decides the exact
    // scope from the real caller and the token's credential ceiling. A browser
    // session is unrestricted; a personal access token contributes only the
    // scopes it carries.
    if !scope_allowed(
        &state.app.authorizer,
        authenticated.session.user.id,
        required_scope,
        &authenticated.kind,
    ) {
        return missing_scope_response(required_scope);
    }

    let response = next.run(request).await;
    if let Some(token_id) = token_id
        && records_successful_use(response.status())
    {
        state.app.personal_access_tokens.record_last_used(token_id);
    }
    response
}

/// The shared engine's decision for one required scope.
///
/// This helper only classifies the credential kind and forwards the actual
/// caller and token ceiling; it never decides scope membership itself.
fn scope_allowed(
    authorizer: &context69_authz::Context69Authorizer,
    user_id: i64,
    required_scope: PersonalAccessTokenScope,
    kind: &AuthKind,
) -> bool {
    let token_scopes = match kind {
        AuthKind::PersonalAccessToken { scopes, .. } => Some(scopes),
        AuthKind::BrowserSession => None,
    };
    authorizer.check(
        user_id,
        context69_authz::Operation::Scope {
            required: required_scope,
            token_scopes,
        },
    )
}

fn missing_scope_response(required_scope: PersonalAccessTokenScope) -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(ApiErrorResponse::new(
            "forbidden",
            format!(
                "personal access token missing {} scope",
                scope_name(required_scope)
            ),
        )),
    )
        .into_response()
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

    use super::scope_allowed;
    use crate::contracts::PersonalAccessTokenScope;

    fn authorizer() -> context69_authz::Context69Authorizer {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(context69_authz::Context69Authorizer::new())
            .expect("engine")
    }

    #[test]
    fn the_shared_engine_decides_present_and_missing_token_scopes() {
        let permitted = super::AuthKind::PersonalAccessToken {
            token_id: Uuid::new_v4(),
            scopes: BTreeSet::from([PersonalAccessTokenScope::Search]),
        };
        assert!(scope_allowed(
            &authorizer(),
            7,
            PersonalAccessTokenScope::Search,
            &permitted
        ));
        assert!(
            !scope_allowed(
                &authorizer(),
                7,
                PersonalAccessTokenScope::Admin,
                &permitted
            ),
            "a missing scope is rejected by the engine, not by a legacy short-circuit"
        );

        assert!(
            scope_allowed(
                &authorizer(),
                7,
                PersonalAccessTokenScope::Search,
                &super::AuthKind::BrowserSession
            ),
            "an authenticated browser session carries every service scope"
        );
    }
}
