//! Viewer-gated exact-file metadata read (issue #681 phase 5B).
//!
//! The route answers one question about one path: does the generation this
//! repository currently serves hold a manifest entry with that path, and what
//! does that entry say? It reuses the same group-scoped source and active
//! generation reads, the same Viewer floor, and the same bounded not-found and
//! not-ready shapes as the manifest listing, so the two reads cannot disagree
//! about which generation they describe and neither can distinguish a path the
//! generation does not hold from a repository that does not exist.
//!
//! This is deliberately a metadata read. The path is validated with the same
//! path-safety validator that admitted the stored entry before the lookup runs,
//! the response carries the unchanged safe `GitRepositoryFile` projection, and
//! nothing here returns file bytes, chunk text, a line or byte range, a
//! provider blob id, or any credential, secret reference, or connection state.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use uuid::Uuid;

use crate::{
    contracts::sources::{GitRepositoryFileDetailResponse, GitRepositoryFileQuery},
    db::{StoredGitGenerationFile, StoredGitRepositoryGeneration, StoredGitRepositorySource},
    domain_errors::DomainError,
    services::git_repository::SafeTreePath,
};

use super::{
    ApiState,
    auth::CurrentUser,
    errors::library_management_error_response,
    git_repository_files::{
        generation_not_ready, repository_not_found, require_manifest_read, serving_generation,
    },
    group_access::{group_access_error_response, group_for_user},
};

#[utoipa::path(
    get,
    path = "/v1/groups/by-path/{group_path}/git-repositories/{repository_key}/file",
    params(
        ("group_path" = String, Path, description = "URL-encoded group path"),
        ("repository_key" = Uuid, Path, description = "Git repository source key"),
        GitRepositoryFileQuery
    ),
    responses(
        (status = 200, description = "Metadata of one exact manifest entry", body = GitRepositoryFileDetailResponse),
        (status = 400, description = "Invalid repository path", body = crate::contracts::ApiErrorResponse),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Group, repository, or path not found"),
        (status = 409, description = "No active ready index generation to read", body = crate::contracts::ApiErrorResponse)
    )
)]
pub(crate) async fn get_git_repository_file(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_path, repository_key)): Path<(String, Uuid)>,
    Query(query): Query<GitRepositoryFileQuery>,
) -> Response {
    let group = match group_for_user(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    if let Err(error) = require_manifest_read(&group) {
        return group_access_error_response(error);
    }
    // The path is validated before the repository is even resolved, so a
    // traversal or control-byte attempt never reaches a lookup and the refusal
    // echoes neither the submitted value nor any repository detail.
    let path = match SafeTreePath::parse(&query.path) {
        Ok(path) => path,
        Err(_) => return invalid_repository_path(),
    };
    let source = match state
        .app
        .db
        .get_git_repository_source(group.id, repository_key)
        .await
    {
        Ok(Some(source)) => source,
        Ok(None) => return repository_not_found(),
        Err(error) => return library_management_error_response(error),
    };
    let generation = match serving_generation(&state.app.db, &source).await {
        Ok(Some(generation)) => generation,
        Ok(None) => return generation_not_ready(),
        Err(error) => return library_management_error_response(error),
    };
    let file = match state
        .app
        .db
        .get_git_generation_file(
            source.group.group_id,
            source.repository_key,
            generation.generation_key,
            path.as_str(),
        )
        .await
    {
        Ok(found) => match found_entry(found) {
            Ok(file) => file,
            Err(response) => return *response,
        },
        Err(error) => return library_management_error_response(error),
    };
    (
        StatusCode::OK,
        Json(file_detail(&source, &generation, file)),
    )
        .into_response()
}

/// The route's single decision about the exact-file lookup result.
///
/// `None` is a path the serving generation does not hold, and it answers with
/// the very same response as an unknown or foreign repository: same status, same
/// body. Keeping that mapping in one named place is what makes it testable
/// without a database and impossible to drift into a distinguishable "no such
/// path" shape, which would let the read enumerate another group's repository
/// keys or indexed paths. The response travels boxed so this stays a plain
/// `Result` rather than a large `Err` variant.
fn found_entry(
    found: Option<StoredGitGenerationFile>,
) -> Result<StoredGitGenerationFile, Box<Response>> {
    found.ok_or_else(|| Box::new(repository_not_found()))
}

/// Compose the response: the exact entry plus the serving generation's
/// provenance and coverage, matching what the manifest page reports for the same
/// generation.
pub(super) fn file_detail(
    source: &StoredGitRepositorySource,
    generation: &StoredGitRepositoryGeneration,
    file: StoredGitGenerationFile,
) -> GitRepositoryFileDetailResponse {
    GitRepositoryFileDetailResponse {
        repository_key: source.repository_key,
        generation_key: generation.generation_key,
        generation_number: generation.generation_number,
        ref_name: generation.ref_name.clone(),
        commit_sha: generation.commit_sha.clone(),
        index_status: source.index_status,
        checkpoint: source.checkpoint.clone(),
        file_count: generation.file_count,
        excluded_file_count: generation.excluded_file_count,
        total_bytes: generation.total_bytes,
        file: file.to_contract(),
    }
}

/// An unsafe path is refused without echoing it: the value never reached a
/// lookup, and the message names no repository, group, or stored path.
fn invalid_repository_path() -> Response {
    library_management_error_response(
        DomainError::invalid_argument("git_repository_path_invalid").into(),
    )
}

#[cfg(test)]
#[path = "git_repository_file_tests.rs"]
mod tests;

/// Database-gated round trips for the same route, reusing the manifest
/// listing's disposable fixture.
#[cfg(test)]
#[path = "git_repository_file_db_tests.rs"]
mod db_tests;
