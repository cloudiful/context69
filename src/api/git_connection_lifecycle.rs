//! Maintainer-gated Git provider connection lifecycle: disable (issue #681 phase
//! 5F) and enable (phase 5G).
//!
//! These are the only lifecycle actions this service exposes: taking an existing
//! group-owned connection out of service and putting it back. Both reuse the
//! group-scoped helpers, the shared Maintainer floor, and the same bounded
//! not-found shape; 5G adds one group-scoped `UPDATE` that clears only
//! `disabled_at`, so it adds no migration, no secret-store operation, and no
//! provider call. Deletion, metadata edits, credential rotation, hook setup,
//! acquisition, and MCP exposure remain outside both phases and need their own
//! plans.
//!
//! Three properties are deliberate:
//!
//! - **The update is the whole mutation, and the ownership test with it.** There
//!   is no read-then-write preflight and no second write path: the update is
//!   group-scoped and reports whether a row matched, so a foreign or unknown key
//!   answers the shared bounded `404` without ever being read. The stored
//!   `COALESCE` is the idempotency and race-safety mechanism, so a repeated
//!   disable is a second success rather than a conflict, and the *first* disable
//!   timestamp is the one that survives.
//! - **The response is the safe projection, read back.** After a matched update
//!   the connection is read once, so the returned `disabled: true` and its
//!   freshness are read from persistence rather than asserted. The projection is
//!   the existing [`GitProviderConnection`] shape: it reports *which* secrets are
//!   configured and never any secret value, key, ciphertext, or reference.
//! - **No secret material is on this path.** The route never opens the secret
//!   store, never reads a credential or signing value, and never calls a
//!   provider, so disabling a connection cannot leak one.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};

use crate::{
    contracts::{MembershipRole, sources::GitProviderConnection},
    domain::GroupRecord,
    domain_errors::DomainError,
};

use super::{
    ApiState,
    auth::CurrentUser,
    errors::library_management_error_response,
    group_access::{group_access_error_response, group_for_user, require_group_role},
};

#[utoipa::path(
    post,
    path = "/v1/groups/by-path/{group_path}/git-connections/{connection_key}/enable",
    params(
        ("group_path" = String, Path, description = "URL-encoded group path"),
        ("connection_key" = String, Path, description = "Git provider connection key")
    ),
    responses(
        (status = 200, description = "The connection, back in service", body = GitProviderConnection),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Group or connection not found", body = crate::contracts::ApiErrorResponse)
    )
)]
pub(crate) async fn enable_git_connection(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_path, connection_key)): Path<(String, String)>,
) -> Response {
    // The same Maintainer floor the create and disable paths enforce: a Viewer or
    // non-member is refused before the update and learns nothing about the
    // connection.
    let group = match maintainer_group(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    // One statement decides everything, exactly as on the disable path: the
    // group-scoped update clears only `disabled_at` and stamps `updated_at`, so
    // the stored credential, App-private-key, and webhook-signing-secret references
    // are untouched, and a repeated enable — or an enable of a connection that was
    // never disabled — reports a match again rather than a conflict. A key that
    // matched no row is a foreign or unknown connection, and it is never read.
    if let Err(response) = enabled_by_update(
        state
            .app
            .db
            .enable_git_provider_connection(group.id, &connection_key)
            .await,
    ) {
        return *response;
    }
    // The projection is read back from persistence, so the caller sees the stored
    // `disabled: false` and the freshness the update wrote, not an assumption.
    let enabled = match state
        .app
        .db
        .get_git_provider_connection(group.id, &connection_key)
        .await
    {
        Ok(Some(connection)) => connection,
        Ok(None) => return connection_not_found(),
        Err(error) => return library_management_error_response(error),
    };
    (StatusCode::OK, Json(enabled.to_contract())).into_response()
}

#[utoipa::path(
    delete,
    path = "/v1/groups/by-path/{group_path}/git-connections/{connection_key}",
    params(
        ("group_path" = String, Path, description = "URL-encoded group path"),
        ("connection_key" = String, Path, description = "Git provider connection key")
    ),
    responses(
        (status = 200, description = "The connection, taken out of service", body = GitProviderConnection),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Group or connection not found", body = crate::contracts::ApiErrorResponse)
    )
)]
pub(crate) async fn disable_git_connection(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_path, connection_key)): Path<(String, String)>,
) -> Response {
    // The same Maintainer floor the create path enforces, resolved through the
    // shared group helpers, so a Viewer or non-member is refused before the
    // mutation and learns nothing about the connection.
    let group = match maintainer_group(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    // One statement decides everything: the group-scoped update is disabled once,
    // the stored `COALESCE` keeps the first timestamp, and a repeat — including a
    // concurrent one — reports a match again, so it is the same successful
    // projection rather than a conflict or a second disable. A key that matched no
    // row is a foreign or unknown connection, and it is never read at all.
    if let Err(response) = disabled_by_update(
        state
            .app
            .db
            .disable_git_provider_connection(group.id, &connection_key)
            .await,
    ) {
        return *response;
    }
    // The projection is read back from persistence, so the caller sees the stored
    // `disabled: true` and the freshness the update wrote, not an assumption.
    let disabled = match state
        .app
        .db
        .get_git_provider_connection(group.id, &connection_key)
        .await
    {
        Ok(Some(connection)) => connection,
        Ok(None) => return connection_not_found(),
        Err(error) => return library_management_error_response(error),
    };
    (StatusCode::OK, Json(disabled.to_contract())).into_response()
}

/// Resolves the group and enforces the Maintainer write role through the existing
/// group-scoped helpers, exactly as the connection create path does.
async fn maintainer_group(
    state: &ApiState,
    user_id: i64,
    group_path: &str,
) -> anyhow::Result<GroupRecord> {
    let group = group_for_user(state, user_id, group_path).await?;
    require_group_role(&group, MembershipRole::Maintainer)?;
    Ok(group)
}

/// The single decision a group-scoped lifecycle update makes, kept under the name
/// the disable arm introduced it with.
///
/// The update is scoped to the owning group, so `false` means no row of this group
/// holds that key: a foreign connection and a key that names nothing are the same
/// case, and both answer the shared bounded `404` the other connection routes
/// return. Deciding it from the update's own result is what keeps the routes free
/// of a preflight read: a foreign key is never read, so it cannot be distinguished
/// from an unknown one by anything a route observes. A storage failure keeps its
/// own mapped error rather than masquerading as a missing connection.
fn disabled_by_update(updated: anyhow::Result<bool>) -> Result<(), Box<Response>> {
    match updated {
        Ok(true) => Ok(()),
        Ok(false) => Err(Box::new(connection_not_found())),
        Err(error) => Err(Box::new(library_management_error_response(error))),
    }
}

/// The enable arm's name for the same mapping, so each route reads as its own
/// decision instead of restating the match.
fn enabled_by_update(updated: anyhow::Result<bool>) -> Result<(), Box<Response>> {
    disabled_by_update(updated)
}

/// A foreign and an unknown connection key share one bounded not-found shape, so
/// this route reveals nothing about another group's connection keys.
fn connection_not_found() -> Response {
    library_management_error_response(
        DomainError::not_found("unknown git provider connection").into(),
    )
}

#[cfg(test)]
#[path = "git_connection_lifecycle_tests.rs"]
mod tests;

/// The enable arm's focused tests, in their own module so neither file crosses the
/// size boundary.
#[cfg(test)]
#[path = "git_connection_enable_tests.rs"]
mod enable_tests;
