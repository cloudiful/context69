//! `search_code` adapter: validate the MCP input, delegate admission and
//! retrieval to the shared code-search service, and project its bounded outcome.
//!
//! HTTP and MCP share one retrieval owner, so the repository must be owned by
//! the caller's resolved group, carry a Viewer-visible role, and have an
//! activated, ready generation before any query runs. Lexical, hybrid, and
//! full-semantic behaviour, fallback metadata, and truncation are decided there;
//! this adapter only enforces the MCP input caps and maps typed failures to
//! bounded MCP errors without forwarding storage or provider text.

use rmcp::ErrorData as McpError;

use super::invalid_params;
use crate::{
    contracts::{
        McpCodeCoverage, McpCodeGeneration, McpCodeHit, McpCodeSearchRequest, McpCodeSearchResponse,
    },
    db::Database,
    domain::{AccessScope, GroupRecord},
    domain_errors::{DomainError, find_domain_error},
    services::{
        git_repository::code_search::{CodeSearchOutcome, CodeSearchParams, search_code},
        settings::SettingsService,
    },
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

/// Run one bounded code search through the shared retrieval service.
pub(crate) async fn search(
    db: &Database,
    settings: &SettingsService,
    group: &GroupRecord,
    scope: &AccessScope,
    request: &McpCodeSearchRequest,
) -> Result<McpCodeSearchResponse, McpError> {
    let params = CodeSearchParams {
        repository_key: request.repository_key,
        query: &request.query,
        path_prefix: request.path_prefix.as_deref(),
        language: request.language.as_deref(),
        limit: i64::from(request.limit),
    };
    let outcome = search_code(db, settings, group, scope, &params)
        .await
        .map_err(search_error)?;
    Ok(project_response(&outcome, usize::from(request.limit)))
}

/// Compose the bounded response: generation provenance, profile metadata, the
/// explicit fallback reason when one applies, and the projected hits.
fn project_response(outcome: &CodeSearchOutcome, limit: usize) -> McpCodeSearchResponse {
    let generation = &outcome.generation;
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
    McpCodeSearchResponse {
        generation: meta,
        requested_profile: outcome.requested_profile,
        effective_profile: outcome.effective_profile,
        vector_unavailable: outcome.vector_unavailable,
        hits: outcome
            .hits
            .iter()
            .map(McpCodeHit::from_search_hit)
            .collect(),
        truncated: outcome.truncated || outcome.hits.len() > limit,
    }
}

/// Map the shared service's typed failures onto bounded MCP errors.
///
/// A missing or foreign repository and a missing or not-ready generation keep
/// their existing resource-not-found shape, a role refusal is a permission
/// error, and everything else is a stable internal error that never forwards a
/// database or provider message.
fn search_error(error: anyhow::Error) -> McpError {
    match find_domain_error(&error) {
        Some(DomainError::NotFound(_)) => {
            McpError::resource_not_found("git repository not found", None)
        }
        Some(DomainError::Conflict(_)) => {
            McpError::resource_not_found("git repository has no active index generation", None)
        }
        Some(DomainError::Forbidden(_)) => {
            McpError::invalid_request("insufficient permissions for group", None)
        }
        Some(DomainError::InvalidArgument(_)) => invalid_params(
            "invalid code search request".to_string(),
            "provide a bounded query and a valid repository key",
        ),
        _ => storage_error(),
    }
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
