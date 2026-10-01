//! Group-scoped Git repository registration and status routes (issue #681
//! work unit 3C2).
//!
//! Registration is public GitHub only and credential-free: the canonical URL
//! is parsed into owner/name, the ref/SHA are validated, and the row is upserted
//! through the existing group-scoped persistence before a `git_index` task is
//! submitted with the exact repository-key payload. Reads return the existing
//! `GitRepositorySource` contract, which already carries identity, target and
//! indexed commits, indexing state, and the active generation pointer, and never
//! blob, chunk, or secret material.

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
        sources::{GitProviderKind, GitRepositoryRegistrationRequest, GitRepositorySource},
    },
    db::NewGitRepositorySource,
    domain_errors::DomainError,
    services::{
        git_repository::{SafeRef, SafeSha, parse_canonical_github_url},
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
        (status = 400, description = "Invalid repository registration", body = crate::contracts::ApiErrorResponse),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Group not found"),
        (status = 409, description = "Idempotency key reuse conflict", body = crate::contracts::ApiErrorResponse)
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
    let source = match validated_source(&request) {
        Ok(source) => source,
        Err(error) => return library_management_error_response(error),
    };
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
    responses(
        (status = 202, description = "Git index task accepted", body = TaskRef),
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
) -> Response {
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
    submit_git_index(&state, &group, session.user.id, repository_key, &headers).await
}

/// Submits the bounded `git_index` task for one already-owned repository key.
async fn submit_git_index(
    state: &ApiState,
    group: &crate::domain::GroupRecord,
    user_id: i64,
    repository_key: Uuid,
    headers: &HeaderMap,
) -> Response {
    submit_task_request(
        state,
        TaskSubmission {
            user_id,
            group_id: Some(group.id),
            group_path: Some(group.group_path.clone()),
            source_key: None,
            kind: TaskKind::GitIndex,
            payloads: vec![serde_json::json!({ "repository_key": repository_key })],
            input_storage_object_ids: Vec::new(),
            idempotency_key: idempotency_key(headers),
        },
    )
    .await
}

/// Validates the request and derives the credential-free GitHub source row.
///
/// Owner/name come from the strict canonical parser, so a mismatched or unsafe
/// URL is rejected before persistence. Branch, ref, and pinned commit are
/// validated with the acquisition safety types, and a branch target ref must
/// match the declared default branch.
fn validated_source(
    request: &GitRepositoryRegistrationRequest,
) -> anyhow::Result<NewGitRepositorySource> {
    let coordinates = parse_canonical_github_url(&request.canonical_url)?;
    SafeRef::parse(&format!("refs/heads/{}", request.default_branch))
        .map_err(|_| DomainError::invalid_argument("git_default_branch_invalid"))?;
    let target_ref = SafeRef::parse(&request.target_ref)?;
    if let Some(branch) = target_ref.as_str().strip_prefix("refs/heads/")
        && branch != request.default_branch
    {
        return Err(DomainError::invalid_argument("git_ref_branch_mismatch").into());
    }
    let pinned_commit = match request.pinned_commit.as_deref() {
        Some(sha) => Some(
            SafeSha::parse(sha)
                .map_err(|_| DomainError::invalid_argument("git_commit_sha_invalid"))?
                .as_str()
                .to_string(),
        ),
        None => None,
    };
    Ok(NewGitRepositorySource {
        connection_key: None,
        provider: GitProviderKind::GitHub,
        canonical_url: format!(
            "https://github.com/{}/{}",
            coordinates.owner, coordinates.name
        ),
        owner: coordinates.owner,
        name: coordinates.name,
        default_branch: request.default_branch.clone(),
        target_ref: target_ref.as_str().to_string(),
        target_commit_sha: pinned_commit,
        index_profile: request.index_profile,
        refresh_policy: request.refresh_policy,
    })
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
