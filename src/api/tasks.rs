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
use context69_contracts::{ApiErrorResponse, ScopeSpec, validate_task_idempotency_key};

use super::{
    ApiState,
    auth::CurrentUser,
    errors::task_error,
    group_access::{group_access_error_response, group_for_user, require_group_role},
};
use crate::domain_errors::DomainError;
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
    responses((status = 200, body = crate::contracts::EnsureScopeResponse), (status = 409, body = ApiErrorResponse))
)]
pub(crate) async fn ensure_scope(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Json(spec): Json<ScopeSpec>,
) -> Response {
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
    user_id: i64,
    group_path: &str,
) -> Result<crate::domain::GroupRecord, Box<Response>> {
    let group = group_for_user(state, user_id, group_path)
        .await
        .map_err(|error| Box::new(group_access_error_response(error)))?;
    require_group_role(&group, crate::contracts::MembershipRole::Maintainer)
        .map_err(|error| Box::new(group_access_error_response(error)))?;
    Ok(group)
}

/// Parse and validate the caller's `Idempotency-Key` header.
///
/// An absent header stays absent, so the ordinary automatic key still applies.
/// A present header must survive the shared contracts rule: the edges are
/// trimmed first and a blank, non-ASCII, internally spaced, slashed or
/// oversized value is refused as `400` before any group, task, database or
/// provider work. The key never becomes absent, and it is never normalized
/// beyond trimming, so a replay sends the same idempotency scope.
fn idempotency_key(headers: &HeaderMap) -> Result<Option<String>, Response> {
    let Some(value) = headers.get("idempotency-key") else {
        return Ok(None);
    };
    let value = value.to_str().map_err(|_| {
        task_error(
            DomainError::invalid_argument(
                "Idempotency-Key must be ASCII text: must contain only ASCII characters [A-Za-z0-9._:-]",
            )
            .into(),
        )
    })?;
    let key = value.trim();
    validate_task_idempotency_key(key).map_err(|error| {
        task_error(
            DomainError::invalid_argument(format!("invalid Idempotency-Key: {error}")).into(),
        )
    })?;
    Ok(Some(key.to_owned()))
}
