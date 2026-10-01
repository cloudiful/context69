//! MCP tool helpers: search, document, code, and source adapters.
//!
//! The `#[tool_router]` registry stays in `super` so the seven frozen tool names
//! remain in one auditable impl block; the per-tool validation, projection,
//! and pagination logic lives here.

pub(crate) mod code;
pub(crate) mod documents;
pub(crate) mod search;
pub(crate) mod sources;

use rmcp::ErrorData as McpError;

/// Build an MCP `invalid_params` error with an agent-actionable fix hint.
pub(crate) fn invalid_params(message: String, fix: &str) -> McpError {
    McpError::invalid_params(message, Some(serde_json::json!({"fix": fix})))
}
