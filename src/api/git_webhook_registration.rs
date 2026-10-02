//! Maintainer-gated Git webhook registration (issue #681 phase 5H).
//!
//! One create-only route under the existing sources scope. A Maintainer submits a
//! provider, the provider-issued hook id, the ownership, and optionally a plain
//! signing secret; the group and the Maintainer floor are resolved *before* any
//! repository or secret work, and a repository the group does not own is the same
//! bounded `404` the other Git routes return.
//!
//! Creation is serialized by a transaction-scoped advisory lock keyed by the
//! owning group and repository, and that order is the point. Under the lock the
//! group-owned registration is pre-checked, so a duplicate answers `409` with no
//! secret-store write at all; a supplied secret is then sealed through the
//! existing [`GitSecretWriter::seal`] seam, which returns the deterministic
//! reference without moving any column; and the create-only insert writes the
//! metadata and that reference together. The broad upsert is deliberately not
//! reused: its conflict clause would overwrite a registration another request owns
//! instead of raising the unique violation this route maps to the same `409`.
//!
//! The deterministic target is the repository record itself, never the caller's
//! hook id, so a submitted id can never aim the write at another record. The
//! response is the existing presence-only `GitWebhookRegistration` projection: no
//! signing secret, ciphertext, store key, derived key, or provider state crosses
//! this path, and the value is never echoed, logged, or returned. Creating a hook
//! at the provider, rotating or deactivating one, processing deliveries, and MCP
//! exposure stay outside this phase.

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
        sources::{GitProviderKind, GitWebhookRegistrationRequest},
    },
    db::{
        Database, NewGitWebhookRegistration, StoredGitRepositorySource,
        StoredGitWebhookRegistration,
    },
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
    git_connection_mutations::is_unique_violation,
    git_repository_files::repository_not_found,
    group_access::{group_access_error_response, group_for_user, require_group_role},
};

#[utoipa::path(
    put,
    path = "/v1/groups/by-path/{group_path}/git-repositories/{repository_key}/webhook",
    params(
        ("group_path" = String, Path, description = "URL-encoded group path"),
        ("repository_key" = Uuid, Path, description = "Git repository source key")
    ),
    request_body = GitWebhookRegistrationRequest,
    responses(
        (status = 201, description = "Created group-owned Git webhook registration", body = crate::contracts::sources::GitWebhookRegistration),
        (status = 400, description = "Invalid hook id or signing secret", body = crate::contracts::ApiErrorResponse),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Group or repository not found"),
        (status = 409, description = "A webhook is already registered, or the hook identity is claimed", body = crate::contracts::ApiErrorResponse)
    )
)]
pub(crate) async fn create_git_webhook_registration(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_path, repository_key)): Path<(String, Uuid)>,
    Json(request): Json<GitWebhookRegistrationRequest>,
) -> Response {
    // The group and its Maintainer floor come first, so a caller without the role
    // learns nothing about the repository and never reaches the secret store.
    let group = match maintainer_group(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    if let Err(rejection) = request.validate_for_create() {
        return invalid_request(rejection.as_str());
    }
    let source = match owned_source(&state, group.id, repository_key).await {
        Ok(Some(source)) => source,
        Ok(None) => return repository_not_found(),
        Err(error) => return library_management_error_response(error),
    };
    if let Err(error) = provider_matches(request.provider, source.provider) {
        return library_management_error_response(error.into());
    }
    match create_group_webhook_registration(
        &state.app.db,
        &state.app.secrets,
        group.id,
        repository_key,
        &request,
    )
    .await
    {
        Ok(stored) => (StatusCode::CREATED, Json(stored.to_contract())).into_response(),
        Err(CreateWebhookFailure::AlreadyExists) => registration_conflict(),
        Err(CreateWebhookFailure::Storage(error)) => library_management_error_response(error),
    }
}

/// What a create attempt failed with before a response was produced.
#[derive(Debug)]
pub(crate) enum CreateWebhookFailure {
    /// The repository already has a registration, or another repository already
    /// claims this provider/hook identity. Both are the same bounded conflict.
    AlreadyExists,
    /// A storage or secret-store failure; the message never carries a value.
    Storage(anyhow::Error),
}

/// Registers one webhook on a repository of `group_id`, sealing a supplied
/// signing secret before the row exists.
///
/// The whole create runs inside the per-repository transaction from
/// [`Database::begin_git_webhook_registration_creation`], so concurrent creates
/// for one repository serialize. Under that lock the group-scoped pre-check
/// rejects a duplicate with no secret-store write, the value is sealed through
/// [`GitSecretWriter::seal`] (which never repoints), and the deterministic
/// reference rides on the single create-only insert. A failed seal leaves no row
/// and a rejected duplicate leaves no sealed value; the only residue of a lost
/// insert race is the store's own reclaimable sealed orphan.
pub(crate) async fn create_group_webhook_registration(
    db: &Database,
    secrets: &SecretStore,
    group_id: i64,
    repository_key: Uuid,
    request: &GitWebhookRegistrationRequest,
) -> Result<StoredGitWebhookRegistration, CreateWebhookFailure> {
    let mut creation = match db
        .begin_git_webhook_registration_creation(group_id, repository_key)
        .await
    {
        Ok(creation) => creation,
        Err(error) => return Err(CreateWebhookFailure::Storage(error)),
    };
    match creation.get(group_id, repository_key).await {
        Ok(Some(_)) => {
            return finish(
                creation.rollback().await,
                Err(CreateWebhookFailure::AlreadyExists),
            );
        }
        Ok(None) => {}
        Err(error) => {
            return finish(
                creation.rollback().await,
                Err(CreateWebhookFailure::Storage(error)),
            );
        }
    }
    let signing_secret_key =
        match seal_signing_secret(db, secrets, group_id, repository_key, request).await {
            Ok(reference) => reference,
            Err(error) => return finish(creation.rollback().await, Err(error)),
        };
    let new = NewGitWebhookRegistration {
        repository_key,
        provider: request.provider,
        external_hook_id: request.external_hook_id.clone(),
        ownership: request.ownership,
        // A supplied secret is what makes a hook verifiable, so an omitted one
        // stores an inactive registration the existing ingress already rejects.
        active: request.is_active(),
        signing_secret_key,
    };
    match creation.insert(group_id, &new).await {
        Ok(stored) => finish(creation.commit().await, Ok(stored)),
        // A duplicate that raced a writer outside this create path still raises
        // the unique violation, and answers with the same bounded conflict as the
        // pre-check instead of an internal error.
        Err(error) if is_unique_violation(&error) => finish(
            creation.rollback().await,
            Err(CreateWebhookFailure::AlreadyExists),
        ),
        Err(error) => finish(
            creation.rollback().await,
            Err(CreateWebhookFailure::Storage(error)),
        ),
    }
}

/// Returns a create result after an explicit lock release, preserving the create
/// outcome when the release itself fails.
///
/// A transaction-scoped advisory lock is released by PostgreSQL when the
/// transaction ends, so a failed release cannot leave the lock behind, but it
/// must still be reported instead of pretending the create completed.
fn finish(
    release: anyhow::Result<()>,
    outcome: Result<StoredGitWebhookRegistration, CreateWebhookFailure>,
) -> Result<StoredGitWebhookRegistration, CreateWebhookFailure> {
    match (release, outcome) {
        (Ok(()), outcome) => outcome,
        (Err(error), Ok(_)) => Err(CreateWebhookFailure::Storage(error)),
        (Err(_), Err(outcome)) => Err(outcome),
    }
}

/// Seals a supplied signing secret and returns its deterministic reference, or
/// `None` when the request carried none.
///
/// The reference is the exact key name the shared store derives for this
/// group-qualified repository record — never one derived from the caller's hook
/// id — so the insert can carry it without a follow-up update. The bytes are
/// sealed untrimmed because a provider signs over the exact configured secret,
/// and no value or key name appears in an error.
async fn seal_signing_secret(
    db: &Database,
    secrets: &SecretStore,
    group_id: i64,
    repository_key: Uuid,
    request: &GitWebhookRegistrationRequest,
) -> Result<Option<String>, CreateWebhookFailure> {
    let Some(value) = request.signing_secret_value() else {
        return Ok(None);
    };
    let target = GitSecretTarget::Repository {
        group_id,
        repository_key,
    };
    match GitSecretWriter::new(db.clone(), secrets.clone())
        .seal(&target, GitSecretSlot::Webhook, Some(value.as_bytes()))
        .await
    {
        Ok(GitSecretWrite::Stored(name)) => Ok(Some(name.as_str().to_string())),
        Ok(GitSecretWrite::Kept) => Err(CreateWebhookFailure::Storage(anyhow::anyhow!(
            "a non-blank signing secret was not sealed"
        ))),
        Err(error) => Err(CreateWebhookFailure::Storage(error)),
    }
}

/// Resolves the group and enforces the Maintainer write role through the existing
/// group-scoped helpers, as the connection create path does.
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
/// reads as absent, so the route cannot be used to enumerate repository keys.
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

/// Refuses a hook whose provider is not the repository's own.
///
/// A hook id is provider-issued, so registering a GitHub id on a GitLab
/// repository would store a row the provider-facing ingress could never resolve.
/// Refusing it here also keeps the mismatch ahead of the seal, so a rejected
/// request writes no secret at all.
fn provider_matches(
    requested: GitProviderKind,
    stored: GitProviderKind,
) -> Result<(), DomainError> {
    if requested != stored {
        return Err(DomainError::conflict("git_webhook_provider_mismatch"));
    }
    Ok(())
}

/// A bounded invalid-argument response that never echoes the submitted value.
fn invalid_request(reason: &'static str) -> Response {
    library_management_error_response(DomainError::invalid_argument(reason).into())
}

/// A second registration on one repository, and a hook identity another
/// repository already claims, share one bounded conflict: neither reveals the
/// existing registration nor repoints it.
fn registration_conflict() -> Response {
    library_management_error_response(
        DomainError::conflict("git_webhook_registration_already_exists").into(),
    )
}

#[cfg(test)]
#[path = "git_webhook_registration_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "git_webhook_registration_db_fixture.rs"]
mod fixture;

#[cfg(test)]
#[path = "git_webhook_registration_db_tests.rs"]
mod db_tests;
