//! Source adapters: safe summaries plus cursor pagination.
//!
//! The MCP surface only ever serializes [`crate::contracts::McpSourceSummary`].
//! Connection names, base queries, origin messages, and database state never
//! leave this module. Listing and resource enumeration share the same integer
//! offset cursor so continuation tokens are interchangeable.

use rmcp::{
    ErrorData as McpError,
    model::{Resource, ResourceContents},
};

use super::invalid_params;
use crate::contracts::{
    McpSourceListArgs, McpSourceListResponse, McpSourceSummary, SourceStatus,
    paginate_source_summaries, parse_offset_cursor,
};

/// Page size for `list_resources` enumeration (cursor is an integer offset).
pub(crate) const RESOURCE_PAGE_SIZE: usize = 50;

/// Project one full source status onto the safe summary.
pub(crate) fn summarize(status: &SourceStatus) -> McpSourceSummary {
    McpSourceSummary::from_status(status)
}

/// Project a status list onto safe summaries.
pub(crate) fn summarize_all(statuses: &[SourceStatus]) -> Vec<McpSourceSummary> {
    statuses.iter().map(summarize).collect()
}

/// Slice safe summaries into one cursor page for `list_sources`.
pub(crate) fn paged_response(
    summaries: &[McpSourceSummary],
    args: &McpSourceListArgs,
) -> Result<McpSourceListResponse, McpError> {
    paginate_source_summaries(summaries, args).map_err(|error| {
        invalid_params(
            error.to_string(),
            "set limit to a value between 1 and 100 and cursor to the next_cursor value",
        )
    })
}

/// Parse a `PaginatedRequestParams` cursor into an integer offset.
pub(crate) fn resource_offset(cursor: Option<&str>) -> Result<usize, McpError> {
    parse_offset_cursor(cursor).map_err(|error| {
        invalid_params(
            error.to_string(),
            "use the next_cursor returned by the previous list_resources call",
        )
    })
}

/// Slice safe summaries into one resource page starting at `start`.
pub(crate) fn resource_page(
    summaries: &[McpSourceSummary],
    start: usize,
) -> (Vec<Resource>, Option<String>) {
    let end = start
        .saturating_add(RESOURCE_PAGE_SIZE)
        .min(summaries.len());
    let resources = summaries
        .iter()
        .skip(start)
        .take(RESOURCE_PAGE_SIZE)
        .map(resource_for)
        .collect();
    let next_cursor = (end < summaries.len()).then(|| end.to_string());
    (resources, next_cursor)
}

/// Build the resource handle for one safe summary.
pub(crate) fn resource_for(summary: &McpSourceSummary) -> Resource {
    Resource::new(
        format!("context69://sources/{}", summary.source_key),
        summary.source_key.clone(),
    )
    .with_description("Configured source checkpoint status")
    .with_mime_type("application/json")
}

/// Serialize one safe summary as resource content.
pub(crate) fn resource_content(
    uri: &str,
    summary: &McpSourceSummary,
) -> Result<ResourceContents, McpError> {
    let content = serde_json::to_string_pretty(summary)
        .map_err(|error| super::super::internal_error(anyhow::Error::new(error)))?;
    Ok(ResourceContents::text(content, uri).with_mime_type("application/json"))
}
