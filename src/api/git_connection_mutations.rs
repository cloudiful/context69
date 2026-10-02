//! Maintainer Git provider connection creation (issue #681 phase 4B3).
//!
//! One create-only route under the existing sources scope. A Maintainer submits
//! bounded non-secret metadata plus a tri-state read-credential patch; a `Set`
//! value is sealed *first* through the shared [`GitSecretWriter::seal`] seam,
//! which returns the deterministic reference without moving any column, and the
//! create-only insert then writes the metadata and that reference together. The
//! route never updates, rotates, clears, enables, disables, deletes, or calls a
//! provider, and it never returns or logs a raw credential.
//!
//! Sealing before the insert is deliberate: the repointing [`GitSecretWriter::write`]
//! attaches a secret to a row that already exists, so using it here would leave
//! a window where an uncredentialed row exists and would make a failed seal
//! indistinguishable from a successful create. Sealing first means a failed
//! seal leaves no row at all, and the only residue of a lost insert race is the
//! store's own reclaimable sealed orphan.
//!
//! The broad connection upsert is deliberately not reused: its conflict clause
//! re-enables a connection, so a create must use the create-only insert, which
//! has no conflict clause and therefore turns a duplicate key into a bounded
//! `409` instead of an overwrite.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};

use crate::{
    contracts::{
        MembershipRole,
        sources::{
            GitProviderConnectionRequest, GitReadCredentialPatch, validate_git_connection_key,
        },
    },
    db::{Database, NewGitProviderConnection, StoredGitProviderConnection},
    domain::GroupRecord,
    domain_errors::DomainError,
    services::{
        git_secrets::{GitSecretSlot, GitSecretTarget, GitSecretWrite, GitSecretWriter},
        secret_store::SecretStore,
    },
};

use super::{
    ApiState,
    auth::CurrentUser,
    errors::library_management_error_response,
    group_access::{group_access_error_response, group_for_user, require_group_role},
};

#[utoipa::path(
    put,
    path = "/v1/groups/by-path/{group_path}/git-connections/{connection_key}",
    params(
        ("group_path" = String, Path, description = "URL-encoded group path"),
        ("connection_key" = String, Path, description = "Git provider connection key")
    ),
    request_body = GitProviderConnectionRequest,
    responses(
        (status = 201, description = "Created group-owned Git provider connection", body = crate::contracts::sources::GitProviderConnection),
        (status = 400, description = "Invalid connection key or request body", body = crate::contracts::ApiErrorResponse),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Group not found"),
        (status = 409, description = "Connection key already exists", body = crate::contracts::ApiErrorResponse)
    )
)]
pub(crate) async fn create_git_connection(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_path, connection_key)): Path<(String, String)>,
    Json(request): Json<GitProviderConnectionRequest>,
) -> Response {
    // The path key is validated before any group or database I/O, per the
    // phase boundary: a malformed key is refused locally and never reaches a
    // group lookup. The bounded response and the authorization behavior that
    // follows are unchanged.
    let connection_key = match validate_git_connection_key(&connection_key) {
        Ok(key) => key,
        Err(rejection) => return invalid_request(rejection.as_str()),
    };
    let group = match maintainer_group(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    if let Err(rejection) = request.validate_for_create() {
        return invalid_request(rejection.as_str());
    }
    match create_group_connection(
        &state.app.db,
        &state.app.secrets,
        group.id,
        &connection_key,
        &request,
    )
    .await
    {
        Ok(stored) => (StatusCode::CREATED, Json(stored.to_contract())).into_response(),
        Err(CreateConnectionFailure::AlreadyExists) => library_management_error_response(
            DomainError::conflict("git_connection_already_exists").into(),
        ),
        Err(CreateConnectionFailure::Storage(error)) => library_management_error_response(error),
    }
}

/// What a create attempt failed with before a response was produced.
#[derive(Debug)]
pub(crate) enum CreateConnectionFailure {
    /// The group already owns a connection with this key.
    AlreadyExists,
    /// A storage or secret-store failure; the message never carries a value.
    Storage(anyhow::Error),
}

/// Creates one group-owned connection and, for a `Set` patch, seals its read
/// credential before the row exists.
///
/// The whole create runs inside the per-key transaction from
/// [`Database::begin_git_connection_creation`], so concurrent creates of the
/// same `(group_id, connection_key)` serialize and a losing request cannot seal
/// after the winner. Order within that lock is still the point: the
/// group-scoped pre-check rejects a duplicate with no secret-store write, the
/// `Set` value is then sealed through [`GitSecretWriter::seal`] (which never
/// repoints), and the returned deterministic reference rides on the single
/// create-only insert. A failed seal leaves no row, and a rejected duplicate
/// leaves no sealed value. The create-only insert has no conflict clause, so a
/// duplicate that races a non-create writer still raises the unique violation
/// mapped to the same bounded `409`; its only residue is the store's
/// reclaimable sealed orphan.
pub(crate) async fn create_group_connection(
    db: &Database,
    secrets: &SecretStore,
    group_id: i64,
    connection_key: &str,
    request: &GitProviderConnectionRequest,
) -> Result<StoredGitProviderConnection, CreateConnectionFailure> {
    let mut creation = match db
        .begin_git_connection_creation(group_id, connection_key)
        .await
    {
        Ok(creation) => creation,
        Err(error) => return Err(CreateConnectionFailure::Storage(error)),
    };

    match creation.get(group_id, connection_key).await {
        Ok(Some(_)) => {
            return finish(
                creation.rollback().await,
                Err(CreateConnectionFailure::AlreadyExists),
            );
        }
        Ok(None) => {}
        Err(error) => {
            return finish(
                creation.rollback().await,
                Err(CreateConnectionFailure::Storage(error)),
            );
        }
    }

    let credential_secret_key = match seal_read_credential(
        db,
        secrets,
        group_id,
        connection_key,
        &request.read_credential,
    )
    .await
    {
        Ok(reference) => reference,
        Err(error) => return finish(creation.rollback().await, Err(error)),
    };

    let new = NewGitProviderConnection {
        connection_key: connection_key.to_string(),
        provider: request.provider,
        mode: request.mode,
        display_name: request.display_name.clone(),
        base_url: request.base_url.clone(),
        credential_secret_key,
        webhook_secret_key: None,
    };
    match creation.insert(group_id, &new).await {
        Ok(stored) => finish(creation.commit().await, Ok(stored)),
        Err(error) if is_unique_violation(&error) => finish(
            creation.rollback().await,
            Err(CreateConnectionFailure::AlreadyExists),
        ),
        Err(error) => finish(
            creation.rollback().await,
            Err(CreateConnectionFailure::Storage(error)),
        ),
    }
}

/// Returns a create result after an explicit lock release, preserving the
/// create outcome when the release itself fails.
///
/// A release failure is a storage failure only when the create otherwise
/// succeeded: a transaction-scoped advisory lock is released by PostgreSQL at
/// the end of the transaction, so an error while committing or rolling back can
/// not leave the lock behind, but it must still be reported instead of
/// pretending the create completed.
fn finish(
    release: anyhow::Result<()>,
    outcome: Result<StoredGitProviderConnection, CreateConnectionFailure>,
) -> Result<StoredGitProviderConnection, CreateConnectionFailure> {
    match (release, outcome) {
        (Ok(()), outcome) => outcome,
        (Err(error), Ok(_)) => Err(CreateConnectionFailure::Storage(error)),
        (Err(_), Err(outcome)) => Err(outcome),
    }
}

/// Seals a `Set` read credential and returns its deterministic reference, or
/// `None` for `Keep`.
///
/// The reference is the exact key name the shared store derives for this
/// group-qualified record, so the insert can carry it without a follow-up
/// update. The supplied bytes are sealed untrimmed; only the blank check looks
/// at whitespace. No value or key name is included in an error.
async fn seal_read_credential(
    db: &Database,
    secrets: &SecretStore,
    group_id: i64,
    connection_key: &str,
    patch: &GitReadCredentialPatch,
) -> Result<Option<String>, CreateConnectionFailure> {
    let Some(value) = patch.set_value() else {
        if patch.is_keep() {
            return Ok(None);
        }
        return Err(CreateConnectionFailure::Storage(anyhow::anyhow!(
            "a create received a credential patch with no value"
        )));
    };
    let target = GitSecretTarget::Connection {
        group_id,
        connection_key: connection_key.to_string(),
    };
    match GitSecretWriter::new(db.clone(), secrets.clone())
        .seal(&target, GitSecretSlot::Credential, Some(value.as_bytes()))
        .await
    {
        Ok(GitSecretWrite::Stored(name)) => Ok(Some(name.as_str().to_string())),
        Ok(GitSecretWrite::Kept) => Err(CreateConnectionFailure::Storage(anyhow::anyhow!(
            "a non-blank credential was not sealed"
        ))),
        Err(error) => Err(CreateConnectionFailure::Storage(error)),
    }
}

/// Resolves the group and enforces the Maintainer write role through the
/// existing group-scoped helpers.
async fn maintainer_group(
    state: &ApiState,
    user_id: i64,
    group_path: &str,
) -> anyhow::Result<GroupRecord> {
    let group = group_for_user(state, user_id, group_path).await?;
    require_group_role(&group, MembershipRole::Maintainer)?;
    Ok(group)
}

/// A bounded invalid-argument response that never echoes the submitted value.
fn invalid_request(reason: &'static str) -> Response {
    library_management_error_response(DomainError::invalid_argument(reason).into())
}

/// Whether an error chain carries a PostgreSQL unique violation (SQLSTATE
/// 23505), so a create race maps to the same bounded conflict as the
/// pre-check instead of an internal error.
pub(crate) fn is_unique_violation(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        matches!(
            cause.downcast_ref::<sqlx::Error>(),
            Some(sqlx::Error::Database(database_error))
                if database_error.code().as_deref() == Some("23505")
        )
    })
}

#[cfg(test)]
#[path = "git_connection_mutation_tests.rs"]
mod tests;
