//! Explicit Git provider connection attachment for repository sources (issue
//! #681 work unit 4A2).
//!
//! Both routes are Maintainer-gated writes under the existing sources scope
//! that change exactly one thing: the `connection_key` reference stored on an
//! already-registered repository source. They never create, configure, enable,
//! or disable a connection, never read the secret store, never call a provider,
//! and never submit an index task — attaching a connection records metadata
//! only, so the credential-free public acquisition path is unchanged.
//!
//! The request body carries one non-secret key that must already exist in the
//! calling group. Group ownership of both the repository and the connection is
//! enforced by the group-scoped reads, so a foreign key reads as the same
//! bounded not-found shape as an unknown one.

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
        sources::{GitRepositoryConnectionRequest, GitRepositorySource},
    },
    db::{NewGitRepositorySource, StoredGitProviderConnection, StoredGitRepositorySource},
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
    put,
    path = "/v1/groups/by-path/{group_path}/git-repositories/{repository_key}/connection",
    params(
        ("group_path" = String, Path, description = "URL-encoded group path"),
        ("repository_key" = Uuid, Path, description = "Git repository source key")
    ),
    request_body = GitRepositoryConnectionRequest,
    responses(
        (status = 200, description = "Repository source with the connection attached", body = GitRepositorySource),
        (status = 400, description = "Invalid connection key", body = crate::contracts::ApiErrorResponse),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Group, repository, or connection not found"),
        (status = 409, description = "Connection is disabled or built for another provider", body = crate::contracts::ApiErrorResponse)
    )
)]
pub(crate) async fn set_git_repository_connection(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_path, repository_key)): Path<(String, Uuid)>,
    Json(request): Json<GitRepositoryConnectionRequest>,
) -> Response {
    let group = match maintainer_group(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    let connection_key = match request.validated_connection_key() {
        Ok(key) => key,
        Err(rejection) => {
            return library_management_error_response(
                DomainError::invalid_argument(rejection.as_str()).into(),
            );
        }
    };
    let source = match owned_source(&state, group.id, repository_key).await {
        Ok(Some(source)) => source,
        Ok(None) => return repository_not_found(),
        Err(error) => return library_management_error_response(error),
    };
    let connection = match owned_connection(&state, group.id, &connection_key).await {
        Ok(Some(connection)) => connection,
        Ok(None) => return connection_not_found(),
        Err(error) => return library_management_error_response(error),
    };
    if let Err(error) = check_attachable(&connection, &source) {
        return library_management_error_response(error.into());
    }
    store_connection(&state, group.id, &source, Some(connection_key)).await
}

#[utoipa::path(
    delete,
    path = "/v1/groups/by-path/{group_path}/git-repositories/{repository_key}/connection",
    params(
        ("group_path" = String, Path, description = "URL-encoded group path"),
        ("repository_key" = Uuid, Path, description = "Git repository source key")
    ),
    responses(
        (status = 200, description = "Repository source with the connection detached", body = GitRepositorySource),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Group or repository not found")
    )
)]
pub(crate) async fn delete_git_repository_connection(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_path, repository_key)): Path<(String, Uuid)>,
) -> Response {
    let group = match maintainer_group(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    let source = match owned_source(&state, group.id, repository_key).await {
        Ok(Some(source)) => source,
        Ok(None) => return repository_not_found(),
        Err(error) => return library_management_error_response(error),
    };
    store_connection(&state, group.id, &source, None).await
}

/// Resolves the group and enforces the Maintainer write role through the
/// existing group-scoped helpers.
///
/// Both routes gate through this one check, so a caller without Maintainer
/// access learns nothing about the repository or the connection beyond the
/// shared forbidden/not-found mapping.
async fn maintainer_group(
    state: &ApiState,
    user_id: i64,
    group_path: &str,
) -> anyhow::Result<GroupRecord> {
    let group = group_for_user(state, user_id, group_path).await?;
    require_group_role(&group, MembershipRole::Maintainer)?;
    Ok(group)
}

/// Reads one repository source owned by `group_id`; a foreign or unknown key
/// reads as absent. A storage failure stays an error instead of masquerading as
/// a missing repository.
async fn owned_source(
    state: &ApiState,
    group_id: i64,
    repository_key: Uuid,
) -> anyhow::Result<Option<StoredGitRepositorySource>> {
    state
        .app
        .db
        .get_git_repository_source(group_id, repository_key)
        .await
}

/// Reads one connection owned by `group_id`; a connection of another group reads
/// as absent, so a foreign key never reveals that it exists.
async fn owned_connection(
    state: &ApiState,
    group_id: i64,
    connection_key: &str,
) -> anyhow::Result<Option<StoredGitProviderConnection>> {
    state
        .app
        .db
        .get_git_provider_connection(group_id, connection_key)
        .await
}

/// Refuses a connection that cannot serve this repository.
///
/// A disabled connection is a lifecycle state the caller must resolve first, and
/// a provider mismatch would point the repository at the wrong provider, so both
/// are conflicts instead of silently stored metadata.
fn check_attachable(
    connection: &StoredGitProviderConnection,
    source: &StoredGitRepositorySource,
) -> Result<(), DomainError> {
    if connection.disabled_at.is_some() {
        return Err(DomainError::conflict("git_connection_disabled"));
    }
    if connection.provider != source.provider {
        return Err(DomainError::conflict("git_connection_provider_mismatch"));
    }
    Ok(())
}

/// Rebuilds the insert-side row for an existing repository with a new
/// connection reference.
///
/// Identity, version, and both policies are copied from the stored row, so an
/// attach or detach can only change the connection reference itself and cannot
/// retarget, re-pin, or re-profile the repository.
fn connection_update(
    source: &StoredGitRepositorySource,
    connection_key: Option<String>,
) -> NewGitRepositorySource {
    NewGitRepositorySource {
        connection_key,
        provider: source.provider,
        canonical_url: source.canonical_url.clone(),
        owner: source.owner.clone(),
        name: source.name.clone(),
        default_branch: source.default_branch.clone(),
        target_ref: source.version.ref_name.clone(),
        target_commit_sha: source.version.commit_sha.clone(),
        index_profile: source.index_profile,
        refresh_policy: source.refresh,
    }
}

/// Writes the connection reference back through the group-scoped upsert, so the
/// update conflicts with — and can only ever be — the same source row.
async fn store_connection(
    state: &ApiState,
    group_id: i64,
    source: &StoredGitRepositorySource,
    connection_key: Option<String>,
) -> Response {
    let update = connection_update(source, connection_key);
    match state
        .app
        .db
        .upsert_git_repository_source(group_id, &update)
        .await
    {
        Ok(stored) => (StatusCode::OK, Json(stored.to_contract())).into_response(),
        Err(error) => library_management_error_response(error),
    }
}

/// A foreign and an unknown repository key share one bounded not-found shape.
fn repository_not_found() -> Response {
    library_management_error_response(DomainError::not_found("unknown git repository").into())
}

/// A foreign and an unknown connection key share one bounded not-found shape, so
/// attaching a key owned by another group reveals nothing.
fn connection_not_found() -> Response {
    library_management_error_response(
        DomainError::not_found("unknown git provider connection").into(),
    )
}

#[cfg(test)]
#[path = "git_repository_connection_tests.rs"]
mod tests;
