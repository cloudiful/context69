//! `search_documents` adapter: MCP-only request in, bounded hits out.
//!
//! The MCP request never carries HTTP compatibility fields (no deprecated
//! `page`, no published window, no metadata filters, no sort mode); it is
//! converted to the internal [`crate::contracts::SearchRequest`] with the page
//! pinned to 1 so cursor pagination stays authoritative.

use rmcp::ErrorData as McpError;

use super::invalid_params;
use crate::contracts::{McpSearchHit, McpSearchRequest, McpSearchResponse, SearchResponse};

/// Validate an MCP search request, mapping failures to `invalid_params`.
pub(crate) fn checked_request(request: &McpSearchRequest) -> Result<(), McpError> {
    request.validate().map_err(|error| {
        invalid_params(
            error.to_string(),
            "set query to 1..=2000 characters and limit to a value between 1 and 20",
        )
    })
}

/// Project a search-service response onto the bounded MCP shape.
///
/// `has_more` is derived from the presence of the service `next_cursor`, so a
/// `has_more = true` response always carries a continuation token by
/// construction. Snippets are capped at 600 characters to match the schema.
pub(crate) fn search_response(response: SearchResponse) -> McpSearchResponse {
    let hits: Vec<McpSearchHit> = response
        .items
        .iter()
        .map(McpSearchHit::from_search_hit)
        .collect();
    McpSearchResponse::new(hits, response.pagination.next_cursor)
}
