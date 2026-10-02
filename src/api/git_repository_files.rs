//! Viewer-gated active-generation manifest listing (issue #681 phase 5A).
//!
//! The route exposes the manifest a snapshot already wrote: one bounded page of
//! safe path entries for the generation the repository currently serves, with
//! that generation's provenance, coverage, and the source's commit checkpoint.
//! It reads only group-scoped rows that exist, so an unknown and a foreign
//! repository share one bounded not-found shape, and a repository with no
//! active ready generation gets one bounded not-ready conflict instead of an
//! empty page that would read like an empty repository.
//!
//! Paging is the shared cursor shape: a bounded `limit` plus the opaque
//! continuation this handler issues, validated and decoded in
//! [`git_repository_file_paging`]. A malformed or out-of-range continuation is
//! rejected before any query runs. No file bytes, chunk text, secret reference,
//! or provider call is part of the path.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use uuid::Uuid;

use crate::{
    contracts::{
        CursorPageQuery, CursorPagination, MembershipRole,
        sources::{GitGenerationStatus, GitRepositoryFileListResponse},
    },
    db::{
        Database, StoredGitGenerationFile, StoredGitRepositoryGeneration, StoredGitRepositorySource,
    },
    domain::GroupRecord,
    domain_errors::DomainError,
};

use super::{
    ApiState,
    auth::CurrentUser,
    errors::library_management_error_response,
    git_repository_file_paging::{ManifestPage, requested_page},
    group_access::{group_access_error_response, group_for_user, require_group_role},
};

#[utoipa::path(
    get,
    path = "/v1/groups/by-path/{group_path}/git-repositories/{repository_key}/files",
    params(
        ("group_path" = String, Path, description = "URL-encoded group path"),
        ("repository_key" = Uuid, Path, description = "Git repository source key"),
        CursorPageQuery
    ),
    responses(
        (status = 200, description = "One page of the active generation's path manifest", body = GitRepositoryFileListResponse),
        (status = 400, description = "Invalid page limit or continuation cursor", body = crate::contracts::ApiErrorResponse),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Group or repository not found"),
        (status = 409, description = "No active ready index generation to read", body = crate::contracts::ApiErrorResponse)
    )
)]
pub(crate) async fn list_git_repository_files(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_path, repository_key)): Path<(String, Uuid)>,
    Query(query): Query<CursorPageQuery>,
) -> Response {
    let group = match group_for_user(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    if let Err(error) = require_manifest_read(&group) {
        return group_access_error_response(error);
    }
    let page = match requested_page(&query) {
        Ok(page) => page,
        Err(error) => return library_management_error_response(error),
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
    // One extra row answers `has_more` without a second count query, and the
    // manifest is read in the same path order the storage layer pages in.
    let mut rows = match state
        .app
        .db
        .list_git_generation_files(
            source.group.group_id,
            source.repository_key,
            generation.generation_key,
            page.fetch_limit(),
            page.offset,
        )
        .await
    {
        Ok(rows) => rows,
        Err(error) => return library_management_error_response(error),
    };
    let has_more = rows.len() as i64 > page.limit();
    if has_more {
        rows.truncate(page.limit() as usize);
    }
    (
        StatusCode::OK,
        Json(manifest_page(&source, &generation, rows, &page, has_more)),
    )
        .into_response()
}

/// The group role this read requires: a manifest is readable by any Viewer.
///
/// The decision is delegated to the shared [`require_group_role`] rather than
/// restated here, so the route cannot drift from the hierarchy every other
/// group-scoped read uses and a refusal stays the shared typed forbidden error.
pub(super) fn require_manifest_read(group: &GroupRecord) -> anyhow::Result<()> {
    require_group_role(group, MembershipRole::Viewer)
}

/// The active generation whose manifest may be read, with its provenance.
///
/// The pointer is group-scoped and only activation writes it, so a repository of
/// another group matches no row. The generation row is read back rather than
/// assumed, and a pointer whose generation is not `Ready` is refused instead of
/// served, so a manifest page is never taken from a generation that is not the
/// one the repository advertises.
pub(super) async fn serving_generation(
    db: &Database,
    source: &StoredGitRepositorySource,
) -> anyhow::Result<Option<StoredGitRepositoryGeneration>> {
    let group_id = source.group.group_id;
    let Some(active) = db
        .get_git_active_generation(group_id, source.repository_key)
        .await?
    else {
        return Ok(None);
    };
    let Some(generation) = db
        .get_git_repository_generation(group_id, source.repository_key, active.generation_key)
        .await?
    else {
        return Ok(None);
    };
    Ok((generation.status == GitGenerationStatus::Ready).then_some(generation))
}

/// Compose the page: safe manifest entries plus the serving generation's
/// provenance, coverage, and continuation. `has_more` is derived from the extra
/// row the read fetched, so a continuation token exists exactly when another
/// page does.
pub(super) fn manifest_page(
    source: &StoredGitRepositorySource,
    generation: &StoredGitRepositoryGeneration,
    rows: Vec<StoredGitGenerationFile>,
    page: &ManifestPage,
    has_more: bool,
) -> GitRepositoryFileListResponse {
    GitRepositoryFileListResponse {
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
        files: rows.into_iter().map(|row| row.to_contract()).collect(),
        pagination: CursorPagination::new(page.next_cursor(has_more), has_more),
    }
}

/// An unknown and a foreign repository key share one bounded not-found shape, so
/// reading a manifest reveals nothing about another group's repository keys.
pub(super) fn repository_not_found() -> Response {
    library_management_error_response(DomainError::not_found("unknown git repository").into())
}

/// A repository with no active ready generation is a conflict, not an empty
/// page: the manifest is never empty just because nothing is indexed yet.
pub(super) fn generation_not_ready() -> Response {
    library_management_error_response(
        DomainError::conflict("git repository has no active index generation").into(),
    )
}

#[cfg(test)]
#[path = "git_repository_files_tests.rs"]
mod tests;

/// Database-gated round trips for the same route: skipped unless
/// `CONTEXT69_TEST_DATABASE_URL` names a migrated scratch database.
#[cfg(test)]
#[path = "git_repository_files_db_tests.rs"]
mod db_tests;
