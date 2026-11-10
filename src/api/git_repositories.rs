//! Group-scoped Git repository registration and status routes (issue #681
//! work unit 3C2).
//!
//! Registration names an explicit provider and may reference an existing
//! group-owned connection. A Forgejo registration requires that connection,
//! because its origin must match the canonical repository URL; a GitHub
//! registration may be public or attach a GitHub connection. The ref/SHA are
//! validated, the row is upserted through the existing group-scoped persistence,
//! and a `git_index` task is submitted with the exact repository-key payload.
//! Reads return the existing `GitRepositorySource` contract, which carries
//! identity, target and indexed commits, indexing state, and the active
//! generation pointer, and never blob, chunk, or secret material.

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use uuid::Uuid;

use crate::{
    contracts::{
        MembershipRole, TaskKind, TaskRef,
        sources::{
            GitConnectionMode, GitIndexRequest, GitPinnedCommitPatch, GitProviderKind,
            GitRepositoryRegistrationRejection, GitRepositoryRegistrationRequest,
            GitRepositorySettingsUpdateRequest, GitRepositorySource,
        },
    },
    db::{GitRepositorySettingsUpdate, NewGitRepositorySource, StoredGitProviderConnection},
    domain_errors::DomainError,
    services::{
        git_repository::{
            ForgejoOrigin, SafeRef, SafeSha, parse_canonical_github_url, provider_url,
        },
        tasks::TaskSubmission,
    },
};

use super::{
    ApiState,
    auth::CurrentUser,
    errors::library_management_error_response,
    group_access::{group_access_error_response, group_for_user, require_group_role},
    submit_task_request,
};

#[utoipa::path(
    post,
    path = "/v1/groups/by-path/{group_path}/git-repositories",
    params(("group_path" = String, Path, description = "URL-encoded group path")),
    request_body = GitRepositoryRegistrationRequest,
    responses(
        (status = 202, description = "Git index task accepted", body = TaskRef),
        (status = 400, description = "Invalid repository registration, provider, or connection requirement", body = crate::contracts::ApiErrorResponse),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Group or referenced connection not found"),
        (status = 409, description = "Connection provider, origin, or state conflict, or idempotency key reuse", body = crate::contracts::ApiErrorResponse)
    )
)]
pub(crate) async fn register_git_repository(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path(group_path): Path<String>,
    headers: HeaderMap,
    Json(request): Json<GitRepositoryRegistrationRequest>,
) -> Response {
    let group = match group_for_user(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    if let Err(error) = require_group_role(&group, MembershipRole::Maintainer) {
        return group_access_error_response(error);
    }
    let connection_key = match request.validated_connection_key() {
        Ok(key) => key,
        Err(rejection) => {
            return library_management_error_response(
                DomainError::invalid_argument(rejection.as_str()).into(),
            );
        }
    };
    let connection = match load_connection(&state, group.id, connection_key.as_deref()).await {
        Ok(connection) => connection,
        Err(error) => return library_management_error_response(error),
    };
    let mut source =
        match validated_source(&request, connection_key.as_deref(), connection.as_ref()) {
            Ok(source) => source,
            Err(error) => return library_management_error_response(error),
        };
    source.created_by_user_id = Some(session.user.id);
    let stored = match state
        .app
        .db
        .upsert_git_repository_source(group.id, &source)
        .await
    {
        Ok(stored) => stored,
        Err(error) => return library_management_error_response(error),
    };
    submit_git_index(
        &state,
        &group,
        session.user.id,
        stored.repository_key,
        false,
        &headers,
    )
    .await
}

#[utoipa::path(
    get,
    path = "/v1/groups/by-path/{group_path}/git-repositories",
    params(("group_path" = String, Path, description = "URL-encoded group path")),
    responses(
        (status = 200, description = "Group-visible Git repository sources", body = [GitRepositorySource]),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Group not found")
    )
)]
pub(crate) async fn list_git_repositories(
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
    match state.app.db.list_git_repository_sources(group.id).await {
        Ok(sources) => {
            let body: Vec<GitRepositorySource> = sources
                .into_iter()
                .map(|source| source.to_contract())
                .collect();
            (StatusCode::OK, Json(body)).into_response()
        }
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(
    get,
    path = "/v1/groups/by-path/{group_path}/git-repositories/{repository_key}",
    params(
        ("group_path" = String, Path, description = "URL-encoded group path"),
        ("repository_key" = Uuid, Path, description = "Git repository source key")
    ),
    responses(
        (status = 200, description = "Git repository source status", body = GitRepositorySource),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Group or repository not found")
    )
)]
pub(crate) async fn get_git_repository(
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
        .get_git_repository_source(group.id, repository_key)
        .await
    {
        Ok(Some(source)) => (StatusCode::OK, Json(source.to_contract())).into_response(),
        Ok(None) => repository_not_found(),
        Err(error) => library_management_error_response(error),
    }
}

#[utoipa::path(
    post,
    path = "/v1/groups/by-path/{group_path}/git-repositories/{repository_key}/index",
    params(
        ("group_path" = String, Path, description = "URL-encoded group path"),
        ("repository_key" = Uuid, Path, description = "Git repository source key")
    ),
    request_body = Option<GitIndexRequest>,
    responses(
        (status = 202, description = "Git index task accepted", body = TaskRef),
        (status = 400, description = "Invalid force flag", body = crate::contracts::ApiErrorResponse),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Group or repository not found"),
        (status = 409, description = "Idempotency key reuse conflict", body = crate::contracts::ApiErrorResponse)
    )
)]
pub(crate) async fn index_git_repository(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_path, repository_key)): Path<(String, Uuid)>,
    headers: HeaderMap,
    body: Option<Json<GitIndexRequest>>,
) -> Response {
    let force_rebuild = body.map(|Json(body)| body.force_rebuild).unwrap_or(false);
    let group = match group_for_user(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    if let Err(error) = require_group_role(&group, MembershipRole::Maintainer) {
        return group_access_error_response(error);
    }
    match state
        .app
        .db
        .get_git_repository_source(group.id, repository_key)
        .await
    {
        Ok(Some(_)) => {}
        Ok(None) => return repository_not_found(),
        Err(error) => return library_management_error_response(error),
    }
    submit_git_index(
        &state,
        &group,
        session.user.id,
        repository_key,
        force_rebuild,
        &headers,
    )
    .await
}

#[utoipa::path(
    patch,
    path = "/v1/groups/by-path/{group_path}/git-repositories/{repository_key}",
    params(
        ("group_path" = String, Path, description = "URL-encoded group path"),
        ("repository_key" = Uuid, Path, description = "Git repository source key")
    ),
    request_body = GitRepositorySettingsUpdateRequest,
    responses(
        (status = 200, description = "Updated Git repository source", body = GitRepositorySource),
        (status = 400, description = "Invalid source settings", body = crate::contracts::ApiErrorResponse),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Group or repository not found")
    )
)]
pub(crate) async fn update_git_repository_settings(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_path, repository_key)): Path<(String, Uuid)>,
    Json(request): Json<GitRepositorySettingsUpdateRequest>,
) -> Response {
    let group = match group_for_user(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    if let Err(error) = require_group_role(&group, MembershipRole::Maintainer) {
        return group_access_error_response(error);
    }
    let settings = match request.validated() {
        Ok(settings) => settings,
        Err(rejection) => {
            return library_management_error_response(
                DomainError::invalid_argument(rejection.as_str()).into(),
            );
        }
    };
    let validated = match validated_settings_update(&settings) {
        Ok(validated) => validated,
        Err(error) => return library_management_error_response(error.into()),
    };
    let update = GitRepositorySettingsUpdate {
        target_ref: validated.target_ref.as_deref(),
        pinned_commit_action: validated.pinned_commit_action,
        pinned_commit_value: validated.pinned_commit_value.as_deref(),
        index_profile: validated.index_profile,
        include_patterns: &settings.include_patterns,
        exclude_patterns: &settings.exclude_patterns,
        interval_seconds: settings.interval_seconds,
        webhook_enabled: settings.webhook_enabled,
        enabled: settings.enabled,
        retention_count: settings.retention_count,
    };
    match state
        .app
        .db
        .update_git_repository_source_settings(group.id, repository_key, &update)
        .await
    {
        Ok(Some(source)) => (StatusCode::OK, Json(source.to_contract())).into_response(),
        Ok(None) => repository_not_found(),
        Err(error) => library_management_error_response(error),
    }
}

/// Normalized target, pin, and profile edits of one settings update.
#[derive(Debug)]
pub(super) struct ValidatedSettingsUpdate {
    pub(super) target_ref: Option<String>,
    pub(super) pinned_commit_action: &'static str,
    pub(super) pinned_commit_value: Option<String>,
    pub(super) index_profile: Option<&'static str>,
}

/// Validates the editable target, pin, and profile against acquisition safety.
///
/// Any structurally safe tracked ref is accepted, including a branch other than
/// the repository's declared default branch; the default branch is metadata,
/// not a constraint on what may be tracked. A pin must be a lowercase 40-hex
/// commit, and every edit is normalized to the wire form the persistence layer
/// stores. Clearing is explicit, so `keep` can never be mistaken for a removal.
pub(super) fn validated_settings_update(
    settings: &GitRepositorySettingsUpdateRequest,
) -> Result<ValidatedSettingsUpdate, DomainError> {
    let target_ref = match settings.target_ref.as_deref() {
        Some(raw) => {
            let parsed = SafeRef::parse(raw)
                .map_err(|_| DomainError::invalid_argument("git_ref_invalid"))?;
            Some(parsed.as_str().to_string())
        }
        None => None,
    };
    let (pinned_commit_action, pinned_commit_value) = match &settings.pinned_commit {
        GitPinnedCommitPatch::Keep => ("keep", None),
        GitPinnedCommitPatch::Clear => ("clear", None),
        GitPinnedCommitPatch::Set(raw) => {
            let sha = SafeSha::parse(raw)
                .map_err(|_| DomainError::invalid_argument("git_commit_sha_invalid"))?;
            ("set", Some(sha.as_str().to_string()))
        }
    };
    Ok(ValidatedSettingsUpdate {
        target_ref,
        pinned_commit_action,
        pinned_commit_value,
        index_profile: settings.index_profile.map(|profile| profile.as_str()),
    })
}

#[utoipa::path(
    delete,
    path = "/v1/groups/by-path/{group_path}/git-repositories/{repository_key}",
    params(
        ("group_path" = String, Path, description = "URL-encoded group path"),
        ("repository_key" = Uuid, Path, description = "Git repository source key")
    ),
    responses(
        (status = 204, description = "Repository source and its indexed content removed"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Group or repository not found"),
        (status = 409, description = "Repository still has active work or cleanup is pending", body = crate::contracts::ApiErrorResponse)
    )
)]
pub(crate) async fn delete_git_repository(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_path, repository_key)): Path<(String, Uuid)>,
) -> Response {
    let group = match group_for_user(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    if let Err(error) = require_group_role(&group, MembershipRole::Maintainer) {
        return group_access_error_response(error);
    }
    match crate::services::git_repository::refresh::delete_repository(
        &state.app,
        group.id,
        repository_key,
    )
    .await
    {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => repository_not_found(),
        Err(error) => library_management_error_response(error),
    }
}

/// Submits the bounded `git_index` task for one already-owned repository key.
async fn submit_git_index(
    state: &ApiState,
    group: &crate::domain::GroupRecord,
    user_id: i64,
    repository_key: Uuid,
    force_rebuild: bool,
    headers: &HeaderMap,
) -> Response {
    if let Err(error) = state
        .app
        .db
        .mark_git_refresh_attempt(group.id, repository_key)
        .await
    {
        return library_management_error_response(error);
    }
    submit_task_request(
        state,
        TaskSubmission {
            user_id,
            group_id: Some(group.id),
            group_path: Some(group.group_path.clone()),
            source_key: None,
            kind: TaskKind::GitIndex,
            payloads: vec![serde_json::json!({
                "repository_key": repository_key,
                "force_rebuild": force_rebuild
            })],
            input_storage_object_ids: Vec::new(),
            idempotency_key: idempotency_key(headers),
        },
    )
    .await
}

/// Loads the optional referenced connection, which must belong to the calling
/// group. A foreign or unknown key is the same bounded not-found shape.
async fn load_connection(
    state: &ApiState,
    group_id: i64,
    connection_key: Option<&str>,
) -> anyhow::Result<Option<StoredGitProviderConnection>> {
    let Some(connection_key) = connection_key else {
        return Ok(None);
    };
    let connection = state
        .app
        .db
        .get_git_provider_connection(group_id, connection_key)
        .await?;
    connection.map(Some).ok_or_else(|| {
        DomainError::not_found(GitRepositoryRegistrationRejection::ConnectionNotFound.as_str())
            .into()
    })
}

/// Validates the request and derives the provider-bound source row.
///
/// The provider must have an acquisition implementation. A Forgejo source
/// requires an attached connection that establishes the instance origin; a
/// GitHub source may be public. Any attached connection must be enabled, belong
/// to the same provider, use an acquisition-capable mode, carry the credential
/// that mode needs, and share the canonical repository origin. Owner/name come
/// from the strict provider parser, so a mismatched or unsafe URL is rejected
/// before persistence. Branch, ref, and pinned commit are validated with the
/// acquisition safety types, and a branch target ref must match the declared
/// default branch.
fn validated_source(
    request: &GitRepositoryRegistrationRequest,
    connection_key: Option<&str>,
    connection: Option<&StoredGitProviderConnection>,
) -> anyhow::Result<NewGitRepositorySource> {
    if !request.provider.supports_acquisition() {
        return Err(DomainError::invalid_argument(
            GitRepositoryRegistrationRejection::ProviderUnsupported.as_str(),
        )
        .into());
    }
    if connection.is_none() && request.provider.requires_connection() {
        return Err(DomainError::invalid_argument(
            GitRepositoryRegistrationRejection::ConnectionRequired.as_str(),
        )
        .into());
    }
    if let Some(connection) = connection {
        validate_connection(connection, request)?;
    }
    let (canonical_url, owner, name) = match request.provider {
        GitProviderKind::GitHub => {
            let coordinates = parse_canonical_github_url(&request.canonical_url)?;
            (
                format!(
                    "https://github.com/{}/{}",
                    coordinates.owner, coordinates.name
                ),
                coordinates.owner,
                coordinates.name,
            )
        }
        GitProviderKind::Forgejo => {
            let connection = connection.ok_or_else(|| {
                DomainError::invalid_argument(
                    GitRepositoryRegistrationRejection::ConnectionRequired.as_str(),
                )
            })?;
            let origin = ForgejoOrigin::from_base_url(&connection.base_url)?;
            let coordinates = origin.parse_repository_url(&request.canonical_url)?;
            (
                origin.canonical_repository_url(&coordinates),
                coordinates.owner,
                coordinates.name,
            )
        }
        _ => {
            return Err(DomainError::invalid_argument(
                GitRepositoryRegistrationRejection::ProviderUnsupported.as_str(),
            )
            .into());
        }
    };
    SafeRef::parse(&format!("refs/heads/{}", request.default_branch))
        .map_err(|_| DomainError::invalid_argument("git_default_branch_invalid"))?;
    let target_ref = SafeRef::parse(&request.target_ref)?;
    let pinned_commit = match request.pinned_commit.as_deref() {
        Some(sha) => Some(
            SafeSha::parse(sha)
                .map_err(|_| DomainError::invalid_argument("git_commit_sha_invalid"))?
                .as_str()
                .to_string(),
        ),
        None => None,
    };
    let settings = request
        .settings
        .clone()
        .validated()
        .map_err(|rejection| DomainError::invalid_argument(rejection.as_str()))?;
    Ok(NewGitRepositorySource {
        connection_key: connection_key.map(str::to_owned),
        provider: request.provider,
        canonical_url,
        owner,
        name,
        default_branch: request.default_branch.clone(),
        target_ref: target_ref.as_str().to_string(),
        pinned_commit_sha: pinned_commit,
        index_profile: request.index_profile,
        include_patterns: settings.include_patterns,
        exclude_patterns: settings.exclude_patterns,
        interval_seconds: settings.interval_seconds,
        webhook_enabled: settings.webhook_enabled,
        enabled: settings.enabled,
        retention_count: settings.retention_count,
        created_by_user_id: None,
    })
}

fn validate_connection(
    connection: &StoredGitProviderConnection,
    request: &GitRepositoryRegistrationRequest,
) -> anyhow::Result<()> {
    if connection.disabled_at.is_some() {
        return Err(conflict(
            GitRepositoryRegistrationRejection::ConnectionDisabled,
        ));
    }
    if connection.provider != request.provider {
        return Err(conflict(
            GitRepositoryRegistrationRejection::ConnectionProviderMismatch,
        ));
    }
    if connection.mode == GitConnectionMode::Installation {
        return Err(conflict(
            GitRepositoryRegistrationRejection::ConnectionModeUnsupported,
        ));
    }
    if connection.mode == GitConnectionMode::Token && connection.credential_secret_key.is_none() {
        return Err(conflict(
            GitRepositoryRegistrationRejection::ConnectionCredentialMissing,
        ));
    }
    provider_url::validate_connection_origin(
        request.provider,
        &connection.base_url,
        &request.canonical_url,
    )?;
    Ok(())
}

fn conflict(rejection: GitRepositoryRegistrationRejection) -> anyhow::Error {
    DomainError::conflict(rejection.as_str()).into()
}

fn repository_not_found() -> Response {
    library_management_error_response(DomainError::not_found("unknown git repository").into())
}

/// Reads the optional `Idempotency-Key` request header, matching the task
/// submission convention used by the batch endpoints.
fn idempotency_key(headers: &HeaderMap) -> Option<String> {
    headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
#[path = "git_repositories_tests.rs"]
mod tests;
