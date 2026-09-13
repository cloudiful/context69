//! Document tool adapters: summaries, bounded detail windows, batch shapes.
//!
//! All detail output uses [`crate::contracts::McpDocumentDetail`] — never the
//! unrestricted [`crate::contracts::DocumentResponse`] — with chunk windows
//! capped at 50 chunks of 4,000 characters each.

use rmcp::ErrorData as McpError;

use super::invalid_params;
use crate::contracts::{
    DocumentResponse, McpBatchDocumentArgs, McpDocumentArgs, McpDocumentDetailResponse,
    McpDocumentKeyArgs, McpDocumentQueryArgs, McpDocumentSummary, paginate_document_detail,
};

/// Default per-item chunk window for `get_documents` batch items.
pub(crate) const BATCH_CHUNKS_PER_ITEM: usize = crate::contracts::MCP_BATCH_CHUNKS_PER_ITEM;
/// Default first-page chunk window for key lookups and resource reads.
pub(crate) const FIRST_PAGE_CHUNKS: usize = crate::contracts::MCP_CHUNK_LIMIT_DEFAULT;

/// Validate `get_document` args and return the requested chunk offset.
pub(crate) fn checked_document_args(args: &McpDocumentArgs) -> Result<usize, McpError> {
    args.validate().map_err(|error| {
        invalid_params(
            error.to_string(),
            "set chunk_limit to a value between 1 and 50",
        )
    })?;
    args.start_offset().map_err(|error| {
        invalid_params(
            error.to_string(),
            "use the next_chunk_cursor returned by get_document",
        )
    })
}

/// Validate `query_documents` args.
pub(crate) fn checked_query_args(args: &McpDocumentQueryArgs) -> Result<(), McpError> {
    args.validate()
        .map_err(|error| invalid_params(error.to_string(), "set limit to a value between 1 and 20"))
}

/// Validate `get_document_by_external_id` args.
pub(crate) fn checked_key_args(args: &McpDocumentKeyArgs) -> Result<(), McpError> {
    args.validate().map_err(|error| {
        invalid_params(
            error.to_string(),
            "provide group_path, key.source_key, and key.external_id",
        )
    })
}

/// Validate `get_documents` args.
pub(crate) fn checked_batch_args(args: &McpBatchDocumentArgs) -> Result<(), McpError> {
    args.validate().map_err(|error| {
        invalid_params(
            error.to_string(),
            "provide one or more document keys (at most 20)",
        )
    })
}

/// Project a full document onto its bounded summary.
pub(crate) fn summary(document: &DocumentResponse) -> McpDocumentSummary {
    McpDocumentSummary::from_document(document)
}

/// Build a bounded detail window for `[start, start + limit)`.
pub(crate) fn detail_window(
    document: &DocumentResponse,
    start: usize,
    limit: usize,
) -> Result<McpDocumentDetailResponse, McpError> {
    paginate_document_detail(document, start, limit).map_err(|error| {
        invalid_params(
            error.to_string(),
            "set chunk_limit to a value between 1 and 50",
        )
    })
}

/// First-page detail window used by key lookups and resource reads.
pub(crate) fn first_page_detail(
    document: &DocumentResponse,
) -> Result<McpDocumentDetailResponse, McpError> {
    detail_window(document, 0, FIRST_PAGE_CHUNKS)
}

/// Batch-item detail window (at most five chunks per item).
pub(crate) fn batch_item_detail(
    document: &DocumentResponse,
) -> Result<McpDocumentDetailResponse, McpError> {
    detail_window(document, 0, BATCH_CHUNKS_PER_ITEM)
}
