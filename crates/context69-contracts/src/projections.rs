//! Bounded MCP-safe projections shared by the MCP protocol adapter.
//!
//! These types are the only document/search/source shapes the MCP surface may
//! serialize. Every array declares `maxItems` and every free-form string
//! declares `maxLength` in its JSON Schema, and the constructors below truncate
//! runtime values to exactly those caps so the schema never over-promises.
//!
//! [`crate::SourceStatus`] (connection names, base queries, origin messages,
//! database state) must never cross this boundary; see [`McpSourceSummary`].

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Default/max result count for `search_documents`.
pub const MCP_SEARCH_LIMIT_DEFAULT: u8 = 8;
/// Hard cap for `search_documents` results per call.
pub const MCP_SEARCH_LIMIT_MAX: u8 = 20;
/// Minimum result count for `search_documents`.
pub const MCP_SEARCH_LIMIT_MIN: u8 = 1;
/// Default/max source entries per `list_sources` call.
pub const MCP_SOURCE_LIMIT_DEFAULT: u8 = 50;
/// Hard cap for `list_sources` entries per call.
pub const MCP_SOURCE_LIMIT_MAX: u8 = 100;
/// Minimum source entries per `list_sources` call.
pub const MCP_SOURCE_LIMIT_MIN: u8 = 1;
/// Default chunk window for `get_document`.
pub const MCP_CHUNK_LIMIT_DEFAULT: usize = 20;
/// Hard cap for chunks returned by one `get_document` call.
pub const MCP_CHUNK_LIMIT_MAX: usize = 50;
/// Minimum chunk window for `get_document`.
pub const MCP_CHUNK_LIMIT_MIN: usize = 1;
/// Hard cap for document keys accepted by one `get_documents` call.
pub const MCP_BATCH_KEYS_MAX: usize = 20;
/// Chunks embedded per item in `get_documents` responses.
pub const MCP_BATCH_CHUNKS_PER_ITEM: usize = 5;
/// Search snippet budget per hit (matches runtime truncation).
pub const MCP_SNIPPET_MAX_CHARS: usize = 600;
/// Chunk text budget per chunk (matches runtime truncation).
pub const MCP_CHUNK_TEXT_MAX_CHARS: usize = 4_000;
/// Longest accepted `query` value for `search_documents`.
pub const MCP_QUERY_MAX_CHARS: usize = 2_000;
/// Longest title/summary/source-uri/external-id kept in MCP projections.
pub const MCP_TITLE_MAX_CHARS: usize = 500;
pub const MCP_SUMMARY_MAX_CHARS: usize = 2_000;
pub const MCP_SOURCE_URI_MAX_CHARS: usize = 2_048;
pub const MCP_EXTERNAL_ID_MAX_CHARS: usize = 512;
pub const MCP_SOURCE_KEY_MAX_CHARS: usize = 256;
pub const MCP_GROUP_PATH_MAX_CHARS: usize = 1_024;
pub const MCP_LOCALE_MAX_CHARS: usize = 64;
pub const MCP_CURSOR_MAX_CHARS: usize = 256;
pub const MCP_DISPLAY_NAME_MAX_CHARS: usize = 256;
pub const MCP_DESCRIPTION_MAX_CHARS: usize = 2_000;

/// Truncate a string to at most `max` characters (never panics on boundaries).
pub fn truncate_chars(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.to_string()
    } else {
        value.chars().take(max).collect()
    }
}

/// Compact search hit: the first step of the progressive search -> detail flow.
///
/// Carries only the identifiers (`document_id` + `external_id`) and the snippet
/// needed to decide whether a bounded `get_document` / `get_documents` call is
/// warranted. Full body text and metadata stay behind the detail operations.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct McpSearchHit {
    pub document_id: i64,
    #[schemars(length(max = 512))]
    pub external_id: String,
    #[schemars(length(max = 500))]
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(max = 2000))]
    pub summary: Option<String>,
    #[schemars(length(max = 2048))]
    pub source_uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_at: Option<DateTime<Utc>>,
    pub score: f32,
    #[schemars(length(max = 600))]
    pub snippet: String,
}

impl McpSearchHit {
    /// Build a hit from a search-service hit, enforcing every schema cap.
    pub fn from_search_hit(hit: &crate::SearchHit) -> Self {
        Self {
            document_id: hit.document_id,
            external_id: truncate_chars(&hit.external_id, MCP_EXTERNAL_ID_MAX_CHARS),
            title: truncate_chars(&hit.title, MCP_TITLE_MAX_CHARS),
            summary: hit
                .summary
                .as_deref()
                .map(|summary| truncate_chars(summary, MCP_SUMMARY_MAX_CHARS)),
            source_uri: truncate_chars(&hit.source_uri, MCP_SOURCE_URI_MAX_CHARS),
            published_at: hit.published_at,
            score: hit.score,
            snippet: truncate_chars(&hit.chunk_text, MCP_SNIPPET_MAX_CHARS),
        }
    }
}

/// One bounded document chunk inside [`McpDocumentDetail`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct McpDocumentChunk {
    pub index: i32,
    #[schemars(length(max = 4000))]
    pub text: String,
}

/// Bounded document detail: the second step of the progressive flow.
///
/// Unlike [`crate::DocumentResponse`] this projection carries no record hash,
/// metadata payload, library internals, or locale/translation bookkeeping —
/// only the identifiers, human-readable header, and the requested chunk window.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct McpDocumentDetail {
    pub document_id: i64,
    #[schemars(length(max = 1024))]
    pub group_path: String,
    #[schemars(length(max = 256))]
    pub source_key: String,
    #[schemars(length(max = 512))]
    pub external_id: String,
    #[schemars(length(max = 500))]
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(max = 2000))]
    pub summary: Option<String>,
    #[schemars(length(max = 2048))]
    pub source_uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    #[schemars(length(max = 50))]
    pub chunks: Vec<McpDocumentChunk>,
}

impl McpDocumentDetail {
    /// Build a header-only detail (no chunks); chunks are attached by the
    /// chunk-window helper in the MCP adapter.
    pub fn header(document: &crate::DocumentResponse) -> Self {
        Self {
            document_id: document.document_id,
            group_path: truncate_chars(&document.group_path, MCP_GROUP_PATH_MAX_CHARS),
            source_key: truncate_chars(&document.source_key, MCP_SOURCE_KEY_MAX_CHARS),
            external_id: truncate_chars(&document.external_id, MCP_EXTERNAL_ID_MAX_CHARS),
            title: truncate_chars(&document.title, MCP_TITLE_MAX_CHARS),
            summary: document
                .summary
                .as_deref()
                .map(|summary| truncate_chars(summary, MCP_SUMMARY_MAX_CHARS)),
            source_uri: truncate_chars(&document.source_uri, MCP_SOURCE_URI_MAX_CHARS),
            published_at: document.published_at,
            updated_at: document.updated_at,
            chunks: Vec::new(),
        }
    }
}

/// Bounded document summary for `query_documents`: identifiers plus header,
/// never chunks or operational fields.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct McpDocumentSummary {
    pub document_id: i64,
    #[schemars(length(max = 512))]
    pub external_id: String,
    #[schemars(length(max = 500))]
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(max = 2000))]
    pub summary: Option<String>,
    #[schemars(length(max = 2048))]
    pub source_uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

impl McpDocumentSummary {
    /// Build a summary from a full document response, enforcing schema caps.
    pub fn from_document(document: &crate::DocumentResponse) -> Self {
        Self {
            document_id: document.document_id,
            external_id: truncate_chars(&document.external_id, MCP_EXTERNAL_ID_MAX_CHARS),
            title: truncate_chars(&document.title, MCP_TITLE_MAX_CHARS),
            summary: document
                .summary
                .as_deref()
                .map(|summary| truncate_chars(summary, MCP_SUMMARY_MAX_CHARS)),
            source_uri: truncate_chars(&document.source_uri, MCP_SOURCE_URI_MAX_CHARS),
            published_at: document.published_at,
            updated_at: document.updated_at,
        }
    }
}

/// Safe source summary: the only source shape the MCP surface may serialize.
///
/// Compared to [`crate::SourceStatus`] this drops `connection`,
/// `has_database_url`, `origin_message`, `sync_strategy`, `connector_type`,
/// `base_query`, `batch_size`, `example_queries`, cursor checkpoints, and
/// success timestamps. Connection names, base queries, origin messages, and
/// database state stay behind an explicit authenticated detail operation that
/// is not anonymous by default (not part of the MCP tool surface).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct McpSourceSummary {
    #[schemars(length(max = 1024))]
    pub group_path: String,
    #[schemars(length(max = 256))]
    pub source_key: String,
    #[schemars(length(max = 256))]
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(max = 2000))]
    pub description: Option<String>,
    pub visibility: crate::Visibility,
    pub origin_status: crate::SourceOriginStatusKind,
}

impl McpSourceSummary {
    /// Project a full source status onto the safe summary, enforcing caps.
    pub fn from_status(status: &crate::SourceStatus) -> Self {
        Self {
            group_path: truncate_chars(&status.group_path, MCP_GROUP_PATH_MAX_CHARS),
            source_key: truncate_chars(&status.source_key, MCP_SOURCE_KEY_MAX_CHARS),
            display_name: truncate_chars(&status.display_name, MCP_DISPLAY_NAME_MAX_CHARS),
            description: status
                .description
                .as_deref()
                .map(|description| truncate_chars(description, MCP_DESCRIPTION_MAX_CHARS)),
            visibility: status.visibility,
            origin_status: status.origin_status.clone(),
        }
    }

    /// Whether this summary is visible to anonymous MCP callers.
    pub fn is_public(&self) -> bool {
        self.visibility == crate::Visibility::Public
    }
}
