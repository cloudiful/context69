//! Viewer-gated metadata-only comparison of two index generations (issue #681
//! phase 5E).
//!
//! This is the first read that looks at two snapshots at once. It reuses the
//! guard chain every other Git read shares — group, the Viewer floor, the
//! group-scoped repository — and then resolves *both* sides of the comparison
//! through the same confined generation reads. An explicit generation key is
//! never trusted as given: it must name a *completed* generation of this
//! repository — the active one, or one a newer snapshot superseded — and a key
//! that does not is indistinguishable from a key that names nothing.
//!
//! The comparison itself is decided in SQL. The two manifests are joined on the
//! repository-relative path, the stored provider content addresses are compared
//! there, and only the resulting change kinds and safe path metadata are
//! selected: no provider blob id, raw blob, chunk text, line or content diff,
//! symbol, credential, secret reference, or connection state exists anywhere on
//! this path. An unchanged path is omitted, so an empty page is the truthful
//! answer for two generations that hold the same bytes at the same paths.
//!
//! Paging is the shared bounded cursor contract, validated and decoded by
//! [`git_repository_file_paging`], so a continuation is a token this service
//! issued and an unchanged path is never reported as a change.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use uuid::Uuid;

use crate::{
    contracts::{
        CursorPageQuery,
        sources::{
            GitGenerationStatus, GitRepositoryFileDiff, GitRepositoryFileDiffFile,
            GitRepositoryFileDiffQuery, GitRepositoryFileDiffResponse,
        },
    },
    db::{
        Database, GitFileDiffSide, StoredGitFileDiff, StoredGitRepositoryGeneration,
        StoredGitRepositorySource,
    },
    domain_errors::DomainError,
};

use super::{
    ApiState,
    auth::CurrentUser,
    errors::library_management_error_response,
    git_repository_file_paging::{ManifestPage, requested_page},
    git_repository_files::{
        generation_not_ready, repository_not_found, require_manifest_read, serving_generation,
    },
    group_access::{group_access_error_response, group_for_user},
};

#[utoipa::path(
    get,
    path = "/v1/groups/by-path/{group_path}/git-repositories/{repository_key}/diff",
    params(
        ("group_path" = String, Path, description = "URL-encoded group path"),
        ("repository_key" = Uuid, Path, description = "Git repository source key"),
        GitRepositoryFileDiffQuery
    ),
    responses(
        (status = 200, description = "One page of changed paths between two index generations", body = GitRepositoryFileDiffResponse),
        (status = 400, description = "Invalid page limit or continuation cursor", body = crate::contracts::ApiErrorResponse),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Group or repository not found"),
        (status = 409, description = "No comparable generation, or the pair runs backwards", body = crate::contracts::ApiErrorResponse)
    )
)]
pub(crate) async fn diff_git_repository_files(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_path, repository_key)): Path<(String, Uuid)>,
    Query(query): Query<GitRepositoryFileDiffQuery>,
) -> Response {
    let group = match group_for_user(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    if let Err(error) = require_manifest_read(&group) {
        return group_access_error_response(error);
    }
    // The page is validated before any repository work, so a malformed
    // continuation never reaches a query and the refusal never echoes it.
    let page = match requested_page(&CursorPageQuery {
        limit: query.limit,
        cursor: query.cursor.clone(),
    }) {
        Ok(page) => page,
        Err(error) => return library_management_error_response(error),
    };
    let source = match found_repository(
        state
            .app
            .db
            .get_git_repository_source(group.id, repository_key)
            .await,
    ) {
        Ok(source) => source,
        Err(response) => return *response,
    };
    // An omitted key resolves to the generation the repository already serves;
    // an explicit one is read back through the same confined lookup, so a caller
    // cannot name an arbitrary UUID, another repository, or a snapshot that is
    // still building or has failed.
    let found_to = match query.to_generation {
        Some(key) => {
            state
                .app
                .db
                .get_git_repository_generation(group.id, repository_key, key)
                .await
        }
        None => serving_generation(&state.app.db, &source).await,
    };
    let to = match found_generation(found_to) {
        Ok(generation) => generation,
        Err(response) => return *response,
    };
    let found_from = match query.from_generation {
        Some(key) => {
            state
                .app
                .db
                .get_git_repository_generation(group.id, repository_key, key)
                .await
        }
        None => indexed_generation(&state.app.db, &source).await,
    };
    let from = match found_generation(found_from) {
        Ok(generation) => generation,
        Err(response) => return *response,
    };
    if let Err(response) = comparable(&from, &to) {
        return *response;
    }
    // One extra row answers `has_more` without a second count query, and the read
    // is ordered by path, the same order the page window addresses.
    let mut rows = match state
        .app
        .db
        .list_git_generation_file_diff(
            group.id,
            repository_key,
            from.generation_key,
            to.generation_key,
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
        Json(diff_page(&source, &from, &to, rows, &page, has_more)),
    )
        .into_response()
}

/// The route's single decision about the group-scoped repository lookup.
///
/// A repository the caller's group does not own, and a repository key that names
/// nothing, both read as `None` and answer with the very same bounded `404` the
/// other Git reads return: same status, same body, no repository, owner, or
/// provider detail, so this read cannot be used to enumerate repository keys. A
/// stored source passes through untouched, and a storage failure keeps its own
/// mapped error.
fn found_repository(
    found: anyhow::Result<Option<StoredGitRepositorySource>>,
) -> Result<StoredGitRepositorySource, Box<Response>> {
    match found {
        Ok(Some(source)) => Ok(source),
        Ok(None) => Err(Box::new(repository_not_found())),
        Err(error) => Err(Box::new(library_management_error_response(error))),
    }
}

/// Whether a generation in this state can be compared.
///
/// A *completed* generation can: the active `ready` one, and a `superseded` one a
/// newer snapshot replaced — which is exactly the state the default starting
/// point lives in, because the generation the checkpoint names becomes
/// superseded the moment a newer commit is indexed. A `building` or `failed`
/// snapshot never can: its manifest is partial or abandoned, so a comparison over
/// it would report a truncated or invented diff.
fn comparable_generation(status: GitGenerationStatus) -> bool {
    matches!(
        status,
        GitGenerationStatus::Ready | GitGenerationStatus::Superseded
    )
}

/// The route's single decision about one side of the comparison.
///
/// The confined read answers with a row of every lifecycle status, so the state
/// decision is made here, next to the response. A key that names a generation
/// which is not comparable answers with the same bounded conflict as a key that
/// names nothing, so a caller can neither compare against a partial or abandoned
/// snapshot nor learn which generation keys exist. A comparable generation passes
/// through untouched, and a storage failure keeps its own mapped error.
fn found_generation(
    found: anyhow::Result<Option<StoredGitRepositoryGeneration>>,
) -> Result<StoredGitRepositoryGeneration, Box<Response>> {
    match found {
        Ok(Some(generation)) if comparable_generation(generation.status) => Ok(generation),
        Ok(Some(_)) | Ok(None) => Err(Box::new(generation_not_ready())),
        Err(error) => Err(Box::new(library_management_error_response(error))),
    }
}

/// A pair that runs backwards is a conflict, not an empty comparison.
///
/// Generation numbers are the repository's own monotonic sequence, so they decide
/// the order without trusting either key. An identical pair is allowed and
/// answers with an empty page, which is truthful: a generation holds the same
/// bytes as itself.
fn comparable(
    from: &StoredGitRepositoryGeneration,
    to: &StoredGitRepositoryGeneration,
) -> Result<(), Box<Response>> {
    if from.generation_number > to.generation_number {
        return Err(Box::new(generations_reversed()));
    }
    Ok(())
}

/// The two generations in the order the caller may compare them.
fn generations_reversed() -> Response {
    library_management_error_response(
        DomainError::conflict("git repository generations are not in compare order").into(),
    )
}

/// The generation the repository says it has indexed, if one is stored for it.
///
/// The default starting point of a comparison is the snapshot behind the
/// source's `indexed_commit_sha`, so the generations of this repository are read
/// and the newest comparable one pinned to that commit is chosen. A repository
/// with no indexed commit, or with a commit no completed generation covers, has
/// nothing to compare from and resolves to `None` — a bounded conflict, never an
/// empty page that would read like "nothing changed".
async fn indexed_generation(
    db: &Database,
    source: &StoredGitRepositorySource,
) -> anyhow::Result<Option<StoredGitRepositoryGeneration>> {
    let generations = db
        .list_git_repository_generations(source.group.group_id, source.repository_key)
        .await?;
    Ok(checkpoint_match(
        &generations,
        source.checkpoint.indexed_commit_sha.as_deref(),
    )
    .cloned())
}

/// The newest comparable generation of `generations` pinned to
/// `indexed_commit_sha`.
///
/// The list arrives newest first, so the first match is the newest one. The
/// generation a checkpoint names is normally `superseded` rather than `ready` —
/// indexing a newer commit completes and activates that snapshot, which is what
/// makes this comparison the interesting one — so a superseded match is the point
/// of this function, while a building or failed snapshot is skipped even when its
/// commit matches, because it is not something to compare against.
fn checkpoint_match<'a>(
    generations: &'a [StoredGitRepositoryGeneration],
    indexed_commit_sha: Option<&str>,
) -> Option<&'a StoredGitRepositoryGeneration> {
    let indexed = indexed_commit_sha?;
    generations.iter().find(|generation| {
        generation.commit_sha == indexed && comparable_generation(generation.status)
    })
}

/// Compose the page: both generations' provenance, both coverage envelopes, the
/// bounded changes, and the continuation.
///
/// `has_more` comes from the extra row the read fetched, so a continuation token
/// exists exactly when another page does, and the unchanged paths the comparison
/// dropped are simply absent rather than reported as changes.
fn diff_page(
    source: &StoredGitRepositorySource,
    from: &StoredGitRepositoryGeneration,
    to: &StoredGitRepositoryGeneration,
    rows: Vec<StoredGitFileDiff>,
    page: &ManifestPage,
    has_more: bool,
) -> GitRepositoryFileDiffResponse {
    GitRepositoryFileDiffResponse {
        repository_key: source.repository_key,
        from_generation_key: from.generation_key,
        from_generation_number: from.generation_number,
        from_ref_name: from.ref_name.clone(),
        from_commit_sha: from.commit_sha.clone(),
        to_generation_key: to.generation_key,
        to_generation_number: to.generation_number,
        to_ref_name: to.ref_name.clone(),
        to_commit_sha: to.commit_sha.clone(),
        index_status: source.index_status,
        checkpoint: source.checkpoint.clone(),
        from_file_count: from.file_count,
        from_excluded_file_count: from.excluded_file_count,
        from_total_bytes: from.total_bytes,
        to_file_count: to.file_count,
        to_excluded_file_count: to.excluded_file_count,
        to_total_bytes: to.total_bytes,
        changes: rows.iter().map(diff_change).collect(),
        pagination: crate::contracts::CursorPagination::new(page.next_cursor(has_more), has_more),
    }
}

/// Project one stored change onto the HTTP contract.
///
/// The two sides are optional because a path one generation does not hold has no
/// entry on that side; each side carries only the safe metadata the storage layer
/// selected, never a provider blob id or content.
fn diff_change(diff: &StoredGitFileDiff) -> GitRepositoryFileDiff {
    GitRepositoryFileDiff {
        path: diff.path.clone(),
        change_kind: diff.change_kind,
        before: diff.before.as_ref().map(diff_side),
        after: diff.after.as_ref().map(diff_side),
    }
}

fn diff_side(side: &GitFileDiffSide) -> GitRepositoryFileDiffFile {
    GitRepositoryFileDiffFile {
        file_key: side.file_key,
        language: side.language.clone(),
        byte_count: side.byte_count,
        line_count: side.line_count,
    }
}

#[cfg(test)]
#[path = "git_repository_file_diff_tests.rs"]
mod tests;

/// Test inputs for the comparison: the in-memory stored types the page is
/// composed from.
#[cfg(test)]
#[path = "git_repository_file_diff_db_fixture.rs"]
mod fixture;

/// Database-gated round trips for the same route: skipped unless
/// `CONTEXT69_TEST_DATABASE_URL` names a migrated scratch database.
#[cfg(test)]
#[path = "git_repository_file_diff_db_tests.rs"]
mod db_tests;
