//! Viewer-gated Git provider connection readiness (issue #681 phase 4B2).
//!
//! The route is an offline, read-only status projection under the existing
//! sources scope. It reads one group-owned connection's already-persisted
//! metadata and derives readiness from it without opening ciphertext, reading a
//! secret, or calling a provider. A foreign and an unknown connection key share
//! one bounded not-found shape, so the route never reveals that another group
//! owns a key.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};

use crate::{
    contracts::{MembershipRole, sources::GitConnectionReadinessResponse},
    domain_errors::DomainError,
    services::git_repository::connection_readiness,
};

use super::{
    ApiState,
    auth::CurrentUser,
    errors::library_management_error_response,
    group_access::{group_access_error_response, group_for_user, require_group_role},
};

#[utoipa::path(
    get,
    path = "/v1/groups/by-path/{group_path}/git-connections/{connection_key}",
    params(
        ("group_path" = String, Path, description = "URL-encoded group path"),
        ("connection_key" = String, Path, description = "Git provider connection key")
    ),
    responses(
        (status = 200, description = "Offline readiness of a group-owned Git provider connection", body = GitConnectionReadinessResponse),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Group or connection not found")
    )
)]
pub(crate) async fn get_git_connection_readiness(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_path, connection_key)): Path<(String, String)>,
) -> Response {
    let group = match group_for_user(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    if let Err(error) = require_group_role(&group, MembershipRole::Viewer) {
        return group_access_error_response(error);
    }
    let connection = match state
        .app
        .db
        .get_git_provider_connection(group.id, &connection_key)
        .await
    {
        Ok(Some(connection)) => connection,
        Ok(None) => return connection_not_found(),
        Err(error) => return library_management_error_response(error),
    };
    (
        StatusCode::OK,
        Json(connection_readiness::response(&connection)),
    )
        .into_response()
}

/// A foreign and an unknown connection key share one bounded not-found shape, so
/// reading readiness reveals nothing about another group's connection keys.
fn connection_not_found() -> Response {
    library_management_error_response(
        DomainError::not_found("unknown git provider connection").into(),
    )
}

#[cfg(test)]
#[path = "git_connection_readiness_tests.rs"]
mod tests;
