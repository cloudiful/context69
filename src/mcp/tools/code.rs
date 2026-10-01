//! `search_code` adapter: validate the MCP input, fail closed on missing or
//! non-active repositories, and project bounded code hits.
//!
//! The repository must be owned by the caller's resolved group and have an
//! activated, ready generation before any query runs, so a missing, foreign, or
//! not-yet-indexed repository is an explicit error instead of an empty result.
//! The lexical query is asked for `limit + 1` hits inside the existing ceiling;
//! the contract keeps at most `limit` and reports the drop through `truncated`.

use rmcp::ErrorData as McpError;

use super::invalid_params;
use crate::{
    contracts::sources::{GitCodeLexicalHit, GitGenerationStatus},
    contracts::{
        McpCodeCoverage, McpCodeGeneration, McpCodeHit, McpCodeSearchRequest, McpCodeSearchResponse,
    },
    db::{Database, GitLexicalCodeSearch, StoredGitRepositoryGeneration},
    domain::AccessScope,
};

/// Validate `search_code` args, mapping failures to `invalid_params`.
pub(crate) fn checked_search_args(request: &McpCodeSearchRequest) -> Result<(), McpError> {
    request.validate().map_err(|error| {
        invalid_params(
            error.to_string(),
            "set group_path, a UUID repository_key, a query of 1..=200 characters, \
             and limit 1..=50",
        )
    })
}

/// Run one bounded lexical code search against a repository's active generation.
pub(crate) async fn search(
    db: &Database,
    group_id: i64,
    scope: &AccessScope,
    request: &McpCodeSearchRequest,
) -> Result<McpCodeSearchResponse, McpError> {
    let source = db
        .get_git_repository_source(group_id, request.repository_key)
        .await
        .map_err(|_| storage_error())?;
    if source.is_none() {
        return Err(McpError::resource_not_found(
            "git repository not found",
            None,
        ));
    }

    let active = db
        .get_git_active_generation(group_id, request.repository_key)
        .await
        .map_err(|_| storage_error())?
        .ok_or_else(|| {
            McpError::resource_not_found("git repository has no active index generation", None)
        })?;

    let generation = db
        .get_git_repository_generation(group_id, request.repository_key, active.generation_key)
        .await
        .map_err(|_| storage_error())?
        .ok_or_else(|| {
            McpError::resource_not_found("git repository active generation is unavailable", None)
        })?;
    if generation.status != GitGenerationStatus::Ready {
        return Err(McpError::resource_not_found(
            "git repository active generation is not ready",
            None,
        ));
    }

    let limit = usize::from(request.limit);
    let fetch_limit = i64::from(request.limit) + 1;
    let hits = db
        .lexical_search_git_generation_chunks(
            group_id,
            &GitLexicalCodeSearch {
                repository_key: request.repository_key,
                query: request.query.clone(),
                path_prefix: request.path_prefix.clone(),
                language: request.language.clone(),
                visible_group_ids: scope.private_group_ids.clone(),
                limit: fetch_limit,
            },
        )
        .await
        .map_err(|_| storage_error())?;

    Ok(project_response(&generation, &hits, limit))
}

/// Assemble the bounded response from the active generation's metadata and the
/// raw hits fetched with `limit + 1`.
fn project_response(
    generation: &StoredGitRepositoryGeneration,
    hits: &[GitCodeLexicalHit],
    limit: usize,
) -> McpCodeSearchResponse {
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
    let projected = hits.iter().map(McpCodeHit::from_lexical_hit).collect();
    McpCodeSearchResponse::new(meta, projected, limit)
}

/// Map an unexpected persistence failure onto a bounded MCP error.
///
/// The database message can name schema internals, so it is replaced with a
/// stable message instead of being forwarded to the caller.
fn storage_error() -> McpError {
    McpError::internal_error(
        "git code search failed".to_string(),
        Some(serde_json::json!({
            "retryable": true,
            "fix": "retry the request; if it persists, check the service logs"
        })),
    )
}
