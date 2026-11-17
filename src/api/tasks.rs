//! Task API handlers, split by responsibility: `submit` owns the enqueue
//! handlers, `lifecycle` the read/mutate handlers, and `streams` the SSE
//! stream. The root keeps the module wiring, the re-exports the router and the
//! OpenAPI document resolve, and the submit plumbing shared with the handlers.

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use context69_contracts::{ApiErrorResponse, ScopeSpec, TokenGroupScope};

use super::{
    ApiState,
    auth::CurrentUser,
    errors::task_error,
    group_access::{group_access_error_response, group_for_user, require_group_role},
};
use crate::services::tasks::TaskSubmission;

mod lifecycle;
mod streams;
mod submit;

pub(crate) use lifecycle::*;
pub(crate) use streams::*;
pub(crate) use submit::*;

#[utoipa::path(
    post,
    path = "/v1/scopes/ensure",
    request_body = ScopeSpec,
    responses((status = 200, description = "Scope ensured and its identifiers returned", body = crate::contracts::EnsureScopeResponse), (status = 409, description = "Scope definition conflicts with existing state", body = ApiErrorResponse))
)]
pub(crate) async fn ensure_scope(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Json(spec): Json<ScopeSpec>,
) -> Response {
    if let Some(restriction) = restricted_token_scope(&session.token_group_scope) {
        let allowed = match state
            .app
            .namespace
            .get_group_for_user(session.user.id, &spec.group_path, None)
            .await
        {
            Ok(group) => group.is_some_and(|group| {
                crate::authorization::group_allowed(Some(restriction), group.id)
            }),
            Err(error) => return task_error(error),
        };
        if !allowed {
            return group_access_error_response(anyhow::anyhow!(
                "insufficient permissions for group"
            ));
        }
    }
    match state.app.tasks.ensure_scope(session.user.id, &spec).await {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(error) => task_error(error),
    }
}

pub(crate) async fn submit_task_request(state: &ApiState, request: TaskSubmission) -> Response {
    match state.app.tasks.submit(request).await {
        Ok(task) => (StatusCode::ACCEPTED, Json(task)).into_response(),
        Err(error) => task_error(error),
    }
}

async fn managed_group(
    state: &ApiState,
    session: &crate::services::auth::AuthSession,
    group_path: &str,
) -> Result<crate::domain::GroupRecord, Box<Response>> {
    let group = group_for_user(state, session, group_path)
        .await
        .map_err(|error| Box::new(group_access_error_response(error)))?;
    require_group_role(&group, crate::contracts::MembershipRole::Maintainer)
        .map_err(|error| Box::new(group_access_error_response(error)))?;
    Ok(group)
}

pub(crate) async fn authorize_task(
    state: &ApiState,
    session: &crate::services::auth::AuthSession,
    task_id: uuid::Uuid,
) -> Result<(), Response> {
    if restricted_token_scope(&session.token_group_scope).is_none() {
        return Ok(());
    }
    let task = state
        .app
        .tasks
        .get(task_id, session.user.id)
        .await
        .map_err(task_error)?;
    require_task_group_allowed(state, session, task.group_path.as_deref()).await
}

pub(crate) async fn require_task_group_allowed(
    state: &ApiState,
    session: &crate::services::auth::AuthSession,
    group_path: Option<&str>,
) -> Result<(), Response> {
    let Some(restriction) = restricted_token_scope(&session.token_group_scope) else {
        return Ok(());
    };
    let Some(group_path) = group_path else {
        return Err(group_access_error_response(
            crate::domain_errors::DomainError::forbidden("insufficient permissions for group")
                .into(),
        ));
    };
    let group = state
        .app
        .namespace
        .get_group_for_user(session.user.id, group_path, None)
        .await
        .map_err(group_access_error_response)?;
    match group {
        Some(group) => crate::authorization::require_group_allowed(Some(restriction), group.id)
            .map_err(group_access_error_response),
        None => Err(group_access_error_response(
            crate::domain_errors::DomainError::forbidden("insufficient permissions for group")
                .into(),
        )),
    }
}

/// The exact-group restriction, or `None` for browser sessions and legacy/all
/// tokens, which keep their historical task behavior.
fn restricted_token_scope(scope: &Option<TokenGroupScope>) -> Option<&TokenGroupScope> {
    scope.as_ref().filter(|scope| scope.is_restricted())
}

fn idempotency_key(headers: &HeaderMap) -> Option<String> {
    headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::restricted_token_scope;
    use crate::contracts::TokenGroupScope;

    #[test]
    fn task_prechecks_apply_only_to_exact_group_ranges() {
        assert!(restricted_token_scope(&None).is_none());
        assert!(restricted_token_scope(&Some(TokenGroupScope::All)).is_none());
        assert_eq!(
            restricted_token_scope(&Some(TokenGroupScope::Groups {
                group_ids: vec![4, 9],
            })),
            Some(&TokenGroupScope::Groups {
                group_ids: vec![4, 9],
            })
        );
    }
}
