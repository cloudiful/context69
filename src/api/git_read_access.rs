//! Narrow read context shared by the Git manifest, exact-file, and content
//! reads.
//!
//! Each read starts the same way: resolve the caller's group, apply the shared
//! Viewer floor, then read the group-owned repository source and the active
//! generation it currently serves. Keeping that chain here means the three
//! reads cannot drift about which generation they describe, and the bounded
//! not-found and not-ready shapes stay identical.

use axum::response::Response;
use uuid::Uuid;

use crate::{
    db::{
        Database, StoredGitGenerationFile, StoredGitRepositoryGeneration, StoredGitRepositorySource,
    },
    domain::GroupRecord,
    domain_errors::DomainError,
};

use super::{
    ApiState,
    errors::library_management_error_response,
    git_repository_files::{
        generation_not_ready, repository_not_found, require_manifest_read, serving_generation,
    },
    group_access::{group_access_error_response, group_for_user},
};

/// The caller's group after the shared manifest-read permission floor.
pub(super) async fn readable_group(
    state: &ApiState,
    user_id: i64,
    group_path: &str,
) -> Result<GroupRecord, Response> {
    let group = match group_for_user(state, user_id, group_path).await {
        Ok(group) => group,
        Err(error) => return Err(group_access_error_response(error)),
    };
    if let Err(error) = require_manifest_read(&group) {
        return Err(group_access_error_response(error));
    }
    Ok(group)
}

/// The group-owned source and the active ready generation it serves.
///
/// A foreign and an unknown repository key share the bounded not-found shape,
/// and a repository with no active ready generation is the bounded conflict.
pub(super) async fn serving_source(
    db: &Database,
    group_id: i64,
    repository_key: Uuid,
) -> Result<(StoredGitRepositorySource, StoredGitRepositoryGeneration), Response> {
    let source = match db.get_git_repository_source(group_id, repository_key).await {
        Ok(Some(source)) => source,
        Ok(None) => return Err(repository_not_found()),
        Err(error) => return Err(library_management_error_response(error)),
    };
    let generation = match serving_generation(db, &source).await {
        Ok(Some(generation)) => generation,
        Ok(None) => return Err(generation_not_ready()),
        Err(error) => return Err(library_management_error_response(error)),
    };
    Ok((source, generation))
}

/// The exact-file lookup result: an absent path answers with the same bounded
/// shape as an unknown or foreign repository, so the read cannot enumerate
/// another group's repository keys or indexed paths. The response travels
/// boxed so this stays a plain `Result` rather than a large `Err` variant.
pub(super) fn found_manifest_file(
    found: Option<StoredGitGenerationFile>,
) -> Result<StoredGitGenerationFile, Box<Response>> {
    found.ok_or_else(|| Box::new(repository_not_found()))
}

/// An unsafe path is refused without echoing it: the value never reached a
/// lookup, and the message names no repository, group, or stored path.
pub(super) fn invalid_repository_path() -> Response {
    library_management_error_response(
        DomainError::invalid_argument("git_repository_path_invalid").into(),
    )
}
