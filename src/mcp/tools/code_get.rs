//! `get_code` adapter: validate the MCP input, fail closed on missing or
//! non-active repositories, and project one bounded, commit-pinned line window
//! of stored UTF-8 text.
//!
//! The guard chain mirrors `search_code`: the repository must be owned by the
//! caller's resolved group and have an activated, ready generation before any
//! manifest lookup, the path must classify as a safe repository-relative tree
//! path, and the exact manifest entry must exist in that generation. Every
//! missing, foreign, not-ready, or absent-path case is the same bounded
//! resource-not-found shape, and storage failures never forward database text.
//!
//! Only chunk text is read: the raw acquisition blob and its provider blob id
//! are never selected, and the window is cut with the same line rule the
//! chunker stored its ranges with, so CRLF endings, trailing whitespace, and
//! UTF-8 boundaries survive verbatim.

use rmcp::ErrorData as McpError;

use super::invalid_params;
use crate::{
    contracts::sources::{
        GitGenerationStatus, MAX_GIT_CONTENT_WINDOW_BYTES, MAX_GIT_CONTENT_WINDOW_LINES,
    },
    contracts::{
        MCP_CODE_GET_CHUNK_LIMIT_MAX, MCP_CODE_GET_CHUNK_LIMIT_MIN, MCP_CODE_LANGUAGE_MAX_CHARS,
        MCP_CODE_PATH_MAX_CHARS, McpCodeCoverage, McpCodeFile, McpCodeGeneration,
        McpGetCodeRequest, McpGetCodeResponse, Visibility, truncate_chars,
    },
    db::{
        Database, StoredGitGenerationChunk, StoredGitGenerationFile, StoredGitRepositoryGeneration,
    },
    domain::AccessScope,
    services::git_repository::SafeTreePath,
};

/// Validate `get_code` args, mapping failures to `invalid_params`.
pub(crate) fn checked_get_args(request: &McpGetCodeRequest) -> Result<(), McpError> {
    request.validate().map_err(|error| {
        let fix = format!(
            "set group_path, a UUID repository_key, a safe repository-relative path, \
             start_line/end_line as positive inclusive lines within {MAX_GIT_CONTENT_WINDOW_LINES}, \
             and chunk_limit {MCP_CODE_GET_CHUNK_LIMIT_MIN}..={MCP_CODE_GET_CHUNK_LIMIT_MAX}"
        );
        invalid_params(error.to_string(), &fix)
    })
}

/// Run one bounded, commit-pinned line-window read against a repository's
/// active generation.
pub(crate) async fn get(
    db: &Database,
    group_id: i64,
    scope: &AccessScope,
    request: &McpGetCodeRequest,
) -> Result<McpGetCodeResponse, McpError> {
    let source = db
        .get_git_repository_source(group_id, request.repository_key)
        .await
        .map_err(|_| storage_error())?
        .ok_or_else(content_not_found)?;
    // A private repository needs its owning group in the caller's scope, the
    // same rule the lexical query enforces through `visible_group_ids`.
    if source.group.visibility == Visibility::Private
        && !scope.private_group_ids.contains(&group_id)
    {
        return Err(content_not_found());
    }

    let active = db
        .get_git_active_generation(group_id, request.repository_key)
        .await
        .map_err(|_| storage_error())?
        .ok_or_else(content_not_found)?;
    let generation = db
        .get_git_repository_generation(group_id, request.repository_key, active.generation_key)
        .await
        .map_err(|_| storage_error())?
        .ok_or_else(content_not_found)?;
    if generation.status != GitGenerationStatus::Ready {
        return Err(content_not_found());
    }

    // The path is shape-checked at the MCP boundary and parsed again here, so an
    // unsafe value never reaches storage and a refusal never echoes it.
    let path = SafeTreePath::parse(&request.path).map_err(|_| invalid_path())?;
    let file = db
        .get_git_generation_file(
            group_id,
            request.repository_key,
            generation.generation_key,
            path.as_str(),
        )
        .await
        .map_err(|_| storage_error())?
        .ok_or_else(content_not_found)?;

    let window = file
        .line_window(
            request.start_line,
            request.end_line,
            MAX_GIT_CONTENT_WINDOW_LINES,
        )
        .map_err(|_| invalid_window())?;
    let chunk_limit = usize::from(request.chunk_limit);
    let fetch_limit = i64::try_from(chunk_limit + 1).map_err(|_| storage_error())?;
    // One extra row is the truncation probe: its presence means another matching
    // chunk exists without a second count query.
    let rows = db
        .list_git_generation_chunks_in_line_range(group_id, &file, &window, fetch_limit, 0)
        .await
        .map_err(|_| storage_error())?;

    Ok(project_response(
        &generation,
        &file,
        window.start_line,
        window.end_line,
        window_page(&rows, window.start_line, window.end_line, chunk_limit),
    ))
}

/// One bounded page of window text: the concatenated chunks that fit, the exact
/// byte count of that text, and the truncation flag.
struct CodeWindowPage {
    text: String,
    byte_count: i64,
    truncated: bool,
}

/// Assemble one bounded window under the shared byte cap and the requested
/// chunk limit.
///
/// Chunks are appended verbatim on their stored boundaries and the byte cap is
/// enforced before the chunk that would cross it, so the text is always a
/// prefix a caller can continue from. One stored chunk is bounded far below the
/// cap, so a page always makes progress.
fn window_page(
    rows: &[StoredGitGenerationChunk],
    start_line: i32,
    end_line: i32,
    chunk_limit: usize,
) -> CodeWindowPage {
    let mut text = String::new();
    // An unconsumed row beyond the requested limit means more chunks exist.
    let mut truncated = rows.len() > chunk_limit;
    for row in rows.iter().take(chunk_limit) {
        let piece = row.text_in_line_window(start_line, end_line);
        if text.len() + piece.len() > MAX_GIT_CONTENT_WINDOW_BYTES {
            truncated = true;
            break;
        }
        text.push_str(&piece);
    }
    CodeWindowPage {
        byte_count: text.len() as i64,
        text,
        truncated,
    }
}

/// Compose the response: bounded generation provenance, the safe manifest entry,
/// the requested bounds, and the assembled page.
fn project_response(
    generation: &StoredGitRepositoryGeneration,
    file: &StoredGitGenerationFile,
    start_line: i32,
    end_line: i32,
    page: CodeWindowPage,
) -> McpGetCodeResponse {
    let meta = McpCodeGeneration::new(
        generation.repository_key,
        generation.generation_key,
        generation.generation_number,
        &generation.ref_name,
        &generation.commit_sha,
        generation.completed_at,
        McpCodeCoverage {
            file_count: generation.file_count,
            excluded_file_count: generation.excluded_file_count,
            total_bytes: generation.total_bytes,
        },
    );
    McpGetCodeResponse {
        generation: meta,
        file: McpCodeFile {
            file_key: file.file_key,
            path: truncate_chars(&file.path, MCP_CODE_PATH_MAX_CHARS),
            language: truncate_chars(&file.language, MCP_CODE_LANGUAGE_MAX_CHARS),
        },
        start_line,
        end_line,
        text: page.text,
        byte_count: page.byte_count,
        truncated: page.truncated,
    }
}

/// One bounded not-found for a missing, foreign, not-ready, or absent-path case,
/// so a caller cannot enumerate another group's repositories or indexed paths.
fn content_not_found() -> McpError {
    McpError::resource_not_found("git repository content not found", None)
}

/// An unsafe path is refused without echoing it and without a storage lookup.
fn invalid_path() -> McpError {
    invalid_params(
        "path must be a safe repository-relative path".to_string(),
        "provide a repository-relative path with no absolute, traversal, control, or backslash segment",
    )
}

/// A line span the storage layer could not accept. It never reached a query.
fn invalid_window() -> McpError {
    invalid_params(
        format!(
            "start_line and end_line must be a positive, ordered window of at most \
             {MAX_GIT_CONTENT_WINDOW_LINES} lines"
        ),
        "set start_line and end_line to inclusive 1-based source lines with end_line >= start_line",
    )
}

/// Map an unexpected persistence failure onto a bounded MCP error.
///
/// The database message can name schema internals, so it is replaced with a
/// stable message instead of being forwarded to the caller.
fn storage_error() -> McpError {
    McpError::internal_error(
        "git code window failed".to_string(),
        Some(serde_json::json!({
            "retryable": true,
            "fix": "retry the request; if it persists, check the service logs"
        })),
    )
}

/// Pure window-assembly and arg-validation tests, kept beside the adapter.
#[cfg(test)]
#[path = "code_get_tests.rs"]
mod tests;
