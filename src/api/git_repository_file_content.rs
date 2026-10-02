//! Viewer-gated bounded line-window content read (issue #681 phase 5C).
//!
//! This is the first repository-content egress path, and it is deliberately
//! small: one exact path, one bounded inclusive line window, and only the stored
//! chunk text that window covers. The guard chain is the 5B chain — group, the
//! shared Viewer floor, group-scoped repository, active ready generation, the
//! same path validator, the same exact-file lookup, and the same bounded
//! not-found and not-ready shapes — so a caller cannot read a byte of a
//! generation the repository does not currently serve, and cannot distinguish a
//! missing repository from a missing path.
//!
//! The text is the stored UTF-8 verbatim: CRLF endings, trailing whitespace, and
//! a missing final newline all survive, and a window is cut with the same line
//! rule the chunker stored its ranges with. Only chunk text is read; the raw
//! acquisition blob and its provider blob id are never selected, and no byte of
//! content outside the requested window is returned.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use uuid::Uuid;

use crate::{
    contracts::{
        CursorPagination,
        sources::{
            GitRepositoryFileContentQuery, GitRepositoryFileContentResponse,
            MAX_GIT_CONTENT_CURSOR_MAX_CHARS, MAX_GIT_CONTENT_WINDOW_BYTES,
            MAX_GIT_CONTENT_WINDOW_LINES,
        },
    },
    db::{
        MAX_GIT_CHUNKS_PER_FILE, StoredGitGenerationChunk, StoredGitGenerationFile,
        StoredGitRepositoryGeneration, StoredGitRepositorySource,
    },
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

/// Chunk rows one read may return.
///
/// The byte cap usually stops a page first, since one stored chunk is far below
/// the cap; this row bound is the storage-side guard on the statement.
const MAX_CHUNK_PAGE_ROWS: i64 = 64;

#[utoipa::path(
    get,
    path = "/v1/groups/by-path/{group_path}/git-repositories/{repository_key}/file/content",
    params(
        ("group_path" = String, Path, description = "URL-encoded group path"),
        ("repository_key" = Uuid, Path, description = "Git repository source key"),
        GitRepositoryFileContentQuery
    ),
    responses(
        (status = 200, description = "Stored text of one bounded line window", body = GitRepositoryFileContentResponse),
        (status = 400, description = "Invalid path, line window, or continuation cursor", body = crate::contracts::ApiErrorResponse),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Group, repository, or path not found"),
        (status = 409, description = "No active ready index generation to read", body = crate::contracts::ApiErrorResponse)
    )
)]
pub(crate) async fn get_git_repository_file_content(
    State(state): State<ApiState>,
    CurrentUser(session): CurrentUser,
    Path((group_path, repository_key)): Path<(String, Uuid)>,
    Query(query): Query<GitRepositoryFileContentQuery>,
) -> Response {
    let group = match group_for_user(&state, session.user.id, &group_path).await {
        Ok(group) => group,
        Err(error) => return group_access_error_response(error),
    };
    if let Err(error) = require_manifest_read(&group) {
        return group_access_error_response(error);
    }
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
    let path = match SafeTreePath::parse(&query.path) {
        Ok(path) => path,
        Err(_) => return invalid_repository_path(),
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
        Ok(found) => match found_content_entry(found) {
            Ok(file) => file,
            Err(response) => return *response,
        },
        Err(error) => return library_management_error_response(error),
    };
    // The window and the continuation are settled before the range query, so a
    // malformed or reversed span never reaches storage.
    let window = match file.line_window(
        query.start_line,
        query.end_line,
        MAX_GIT_CONTENT_WINDOW_LINES,
    ) {
        Ok(window) => window,
        Err(error) => return library_management_error_response(error),
    };
    let offset = match decode_cursor(query.cursor.as_deref()) {
        Ok(offset) => offset,
        Err(response) => return *response,
    };
    // One extra row is the continuation probe: its presence means another
    // matching chunk exists without a second count query.
    let rows = match state
        .app
        .db
        .list_git_generation_chunks_in_line_range(
            source.group.group_id,
            &file,
            &window,
            MAX_CHUNK_PAGE_ROWS + 1,
            offset,
        )
        .await
    {
        Ok(rows) => rows,
        Err(error) => return library_management_error_response(error),
    };
    let page = content_page(&rows, window.start_line, window.end_line, offset);
    (
        StatusCode::OK,
        Json(file_content(
            &source,
            &generation,
            file,
            window.start_line,
            window.end_line,
            page,
        )),
    )
        .into_response()
}

/// The route's single decision about the exact-file lookup result.
///
/// `None` is a path the serving generation does not hold, and it answers with the
/// very same response as an unknown or foreign repository: same status, same
/// body. Keeping that mapping in one named place is what makes it testable
/// without a database and impossible to drift into a distinguishable "no such
/// path" shape, which would let this read enumerate another group's repository
/// keys or indexed paths. The response travels boxed so this stays a plain
/// `Result` rather than a large `Err` variant.
fn found_content_entry(
    found: Option<StoredGitGenerationFile>,
) -> Result<StoredGitGenerationFile, Box<Response>> {
    found.ok_or_else(|| Box::new(repository_not_found()))
}

/// One page of window text: the concatenated chunks that fit, the exact byte
/// count of that text, and the continuation.
struct ContentPage {
    text: String,
    byte_count: i64,
    pagination: CursorPagination,
}

/// Assemble one page under the byte cap.
///
/// Chunks are appended verbatim and the cap is enforced *before* the chunk that
/// would cross it, so the page always ends on a chunk boundary and a caller can
/// concatenate continuation pages byte for byte. One stored chunk is bounded
/// far below the cap by the storage ceiling, so a page always makes progress and
/// continuation terminates.
fn content_page(
    rows: &[StoredGitGenerationChunk],
    start_line: i32,
    end_line: i32,
    offset: i64,
) -> ContentPage {
    let mut text = String::new();
    let mut consumed = 0_i64;
    // An unconsumed row beyond the page bound means more matching chunks exist.
    let mut has_more = rows.len() as i64 > MAX_CHUNK_PAGE_ROWS;
    for row in rows.iter().take(MAX_CHUNK_PAGE_ROWS as usize) {
        let piece = row.text_in_line_window(start_line, end_line);
        if text.len() + piece.len() > MAX_GIT_CONTENT_WINDOW_BYTES {
            has_more = true;
            break;
        }
        text.push_str(&piece);
        consumed += 1;
    }
    let next_cursor = has_more.then(|| (offset + consumed).to_string());
    ContentPage {
        byte_count: text.len() as i64,
        text,
        pagination: CursorPagination::new(next_cursor, has_more),
    }
}

/// Compose the response: the exact text, its byte count, the manifest entry, the
/// serving generation's provenance and coverage, and the requested bounds.
fn file_content(
    source: &StoredGitRepositorySource,
    generation: &StoredGitRepositoryGeneration,
    file: StoredGitGenerationFile,
    start_line: i32,
    end_line: i32,
    page: ContentPage,
) -> GitRepositoryFileContentResponse {
    GitRepositoryFileContentResponse {
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
        start_line,
        end_line,
        byte_count: page.byte_count,
        text: page.text,
        pagination: page.pagination,
    }
}

/// Decode the server-issued continuation: a decimal offset into the matching
/// chunk rows of this same window.
///
/// A continuation carries no signed or caller-chosen state, so only a bounded
/// decimal offset is accepted, and one file holds at most
/// `MAX_GIT_CHUNKS_PER_FILE` chunks, so a larger offset could never be one this
/// service issued.
fn decode_cursor(cursor: Option<&str>) -> Result<i64, Box<Response>> {
    let Some(cursor) = cursor else {
        return Ok(0);
    };
    if cursor.is_empty()
        || cursor.len() > MAX_GIT_CONTENT_CURSOR_MAX_CHARS
        || !cursor.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(Box::new(invalid_cursor()));
    }
    match cursor
        .parse::<i64>()
        .ok()
        .filter(|offset| (0..=MAX_GIT_CHUNKS_PER_FILE as i64).contains(offset))
    {
        Some(offset) => Ok(offset),
        None => Err(Box::new(invalid_cursor())),
    }
}

/// An unsafe path is refused without echoing it: the value never reached a
/// lookup, and the message names no repository, group, or stored path.
fn invalid_repository_path() -> Response {
    library_management_error_response(
        DomainError::invalid_argument("git_repository_path_invalid").into(),
    )
}

/// A continuation this service did not issue.
fn invalid_cursor() -> Response {
    library_management_error_response(
        DomainError::invalid_argument("git_content_cursor_invalid").into(),
    )
}

/// Route tests for the content read: authorization, refusals, the exact-file
/// mapping, and the response the route composes.
#[cfg(test)]
#[path = "git_repository_file_content_tests.rs"]
mod tests;

/// Pure line-window, page, and continuation tests for the same read, split into
/// their own module so each test file stays cohesive.
#[cfg(test)]
#[path = "git_repository_file_content_window_tests.rs"]
mod window_tests;

/// Database-gated round trips for the same route, reusing the manifest
/// listing's disposable fixture.
#[cfg(test)]
#[path = "git_repository_file_content_db_tests.rs"]
mod db_tests;
