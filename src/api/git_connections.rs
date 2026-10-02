//! Read-only Git provider connection and webhook status routes (issue #681
//! work unit 4A1).
//!
//! Both routes are viewer-gated reads under the existing sources scope. They
//! expose only the already-persisted, group-scoped metadata: the provider
//! connection summaries and one repository's webhook registration status. They
//! never accept credentials, call a provider, mutate a connection or webhook, or
//! read secret material — the existing contract projections report presence
//! flags only, so no secret value or secret-store key crosses the API.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use uuid::Uuid;

use crate::{
    contracts::{
        MembershipRole,
        sources::{GitProviderConnection, GitWebhookRegistration},
    },
    domain_errors::DomainError,
};

use super::{
    ApiState,
    auth::CurrentUser,
    errors::library_management_error_response,
    group_access::{group_access_error_response, group_for_user, require_group_role},
};

#[utoipa::path(
    get,
    path = "/v1/groups/by-path/{group_path}/git-connections",
    params(("group_path" = String, Path, description = "URL-encoded group path")),
    responses(
        (status = 200, description = "Group-visible Git provider connections", body = [GitProviderConnection]),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Group not found")
    )
)]
pub(crate) async fn list_git_provider_connections(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(group_path): Path<String>,
) -> Response {
    let group = match group_for_user(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    if let Err(error) = require_group_role(&group, MembershipRole::Viewer) {
        return group_access_error_response(error);
    }
    match state.app.db.list_git_provider_connections(group.id).await {
        Ok(connections) => {
            let body: Vec<GitProviderConnection> = connections
                .into_iter()
                .map(|connection| connection.to_contract())
                .collect();
            (StatusCode::OK, Json(body)).into_response()
        }
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(
    get,
    path = "/v1/groups/by-path/{group_path}/git-repositories/{repository_key}/webhook",
    params(
        ("group_path" = String, Path, description = "URL-encoded group path"),
        ("repository_key" = Uuid, Path, description = "Git repository source key")
    ),
    responses(
        (status = 200, description = "Webhook registration status of a group-owned repository", body = GitWebhookRegistration),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Group, repository, or webhook registration not found")
    )
)]
pub(crate) async fn get_git_repository_webhook(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_path, repository_key)): Path<(String, Uuid)>,
) -> Response {
    let group = match group_for_user(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    if let Err(error) = require_group_role(&group, MembershipRole::Viewer) {
        return group_access_error_response(error);
    }
    match state
        .app
        .db
        .get_git_webhook_registration(group.id, repository_key)
        .await
    {
        Ok(Some(registration)) => {
            (StatusCode::OK, Json(registration.to_contract())).into_response()
        }
        Ok(None) => webhook_not_found(),
        Err(error) => library_management_error_response(error),
    }
}

/// A foreign or unknown repository key and a repository without a registration
/// both read as the same bounded not-found shape, so the route never reveals
/// whether another group owns the key.
fn webhook_not_found() -> Response {
    library_management_error_response(
        DomainError::not_found("unknown git webhook registration").into(),
    )
}

#[cfg(test)]
#[path = "git_connection_tests.rs"]
mod tests;
