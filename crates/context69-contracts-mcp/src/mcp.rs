//! MCP-only tool inputs and results.
//!
//! These types are the MCP protocol adapter boundary: tool inputs never reuse
//! HTTP request DTOs (no deprecated `page`, no HTTP pagination or metadata
//! filter surface), and tool outputs use only the bounded projections in
//! [`crate::projections`]. Every `has_more = true` response carries a
//! continuation cursor (`next_cursor` / `next_chunk_cursor`) by construction.
//!
//! Frozen tool names: `search_documents`, `get_document`, `query_documents`,
//! `get_document_by_external_id`, `get_documents`, `list_sources`.

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::projections::{
    MCP_CHUNK_LIMIT_DEFAULT, MCP_CHUNK_LIMIT_MAX, MCP_CHUNK_LIMIT_MIN, MCP_CURSOR_MAX_CHARS,
    MCP_GROUP_PATH_MAX_CHARS, MCP_LOCALE_MAX_CHARS, MCP_QUERY_MAX_CHARS, MCP_SEARCH_LIMIT_DEFAULT,
    MCP_SEARCH_LIMIT_MAX, MCP_SEARCH_LIMIT_MIN, MCP_SOURCE_KEY_MAX_CHARS, MCP_SOURCE_LIMIT_DEFAULT,
    MCP_SOURCE_LIMIT_MAX, MCP_SOURCE_LIMIT_MIN, McpDocumentChunk, McpDocumentDetail,
    McpDocumentSummary, McpSearchHit, McpSourceSummary, truncate_chars,
};

/// Frozen MCP tool names. The registry in `src/mcp` must expose exactly these.
pub const MCP_TOOL_NAMES: [&str; 6] = [
    "search_documents",
    "get_document",
    "query_documents",
    "get_document_by_external_id",
    "get_documents",
    "list_sources",
];

/// MCP-local default result window for `query_documents` when the nested
/// `query.limit` is omitted. The HTTP [`context69_contracts_search::DocumentQueryRequest`] default
/// (50) never applies to tool calls and stays untouched for HTTP wire
/// compatibility; an explicitly supplied `query.limit` must still be 1..=20.
pub const MCP_QUERY_LIMIT_DEFAULT: u8 = 20;

fn default_mcp_search_limit() -> u8 {
    MCP_SEARCH_LIMIT_DEFAULT
}

fn default_mcp_source_limit() -> u8 {
    MCP_SOURCE_LIMIT_DEFAULT
}

fn default_mcp_query_limit() -> u8 {
    MCP_QUERY_LIMIT_DEFAULT
}

fn default_chunk_limit() -> usize {
    MCP_CHUNK_LIMIT_DEFAULT
}

fn non_blank(value: &str) -> bool {
    !value.trim().is_empty()
}

/// MCP-only search input.
///
/// Intentionally narrower than the HTTP [`context69_contracts_search::SearchRequest`]: no deprecated
/// `page`, no `published_after`/`published_before`, no `metadata_filters`, no
/// `sort` mode. Structured filtering stays behind `query_documents`; search is
/// relevance retrieval with an opaque continuation cursor.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpSearchRequest {
    #[schemars(length(min = 1, max = 2000))]
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(max = 1024))]
    pub group_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(max = 256))]
    pub source_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(max = 64))]
    pub locale: Option<String>,
    #[serde(default = "default_mcp_search_limit")]
    #[schemars(range(min = 1, max = 20))]
    pub limit: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(max = 256))]
    pub cursor: Option<String>,
}

impl McpSearchRequest {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.query.trim().is_empty() {
            return Err(anyhow::anyhow!("query must not be blank"));
        }
        if self.query.chars().count() > MCP_QUERY_MAX_CHARS {
            return Err(anyhow::anyhow!("query must be at most 2000 characters"));
        }
        if self.limit < MCP_SEARCH_LIMIT_MIN || self.limit > MCP_SEARCH_LIMIT_MAX {
            return Err(anyhow::anyhow!("limit must be between 1 and 20"));
        }
        if let Some(group_path) = self.group_path.as_deref()
            && (!non_blank(group_path) || group_path.chars().count() > MCP_GROUP_PATH_MAX_CHARS)
        {
            return Err(anyhow::anyhow!("group_path must be 1..=1024 characters"));
        }
        if let Some(source_key) = self.source_key.as_deref()
            && (!non_blank(source_key) || source_key.chars().count() > MCP_SOURCE_KEY_MAX_CHARS)
        {
            return Err(anyhow::anyhow!("source_key must be 1..=256 characters"));
        }
        if let Some(locale) = self.locale.as_deref()
            && (!non_blank(locale) || locale.chars().count() > MCP_LOCALE_MAX_CHARS)
        {
            return Err(anyhow::anyhow!("locale must be 1..=64 characters"));
        }
        if let Some(cursor) = self.cursor.as_deref()
            && (!non_blank(cursor) || cursor.chars().count() > MCP_CURSOR_MAX_CHARS)
        {
            return Err(anyhow::anyhow!("cursor must be 1..=256 characters"));
        }
        Ok(())
    }

    /// Convert to the internal HTTP search DTO. The deprecated `page` is pinned
    /// to 1 (cursor pagination is authoritative) and HTTP-only filter surface
    /// is left empty.
    pub fn to_search_request(&self) -> context69_contracts_search::SearchRequest {
        context69_contracts_search::SearchRequest {
            query: self.query.clone(),
            locale: self.locale.clone(),
            limit: usize::from(self.limit),
            page: 1,
            source_key: self.source_key.clone(),
            group_path: self.group_path.clone(),
            published_after: None,
            published_before: None,
            cursor: self.cursor.clone(),
            metadata_filters: Vec::new(),
            sort: context69_contracts_search::SearchSort::Relevance,
        }
    }
}

/// Bounded search output. `has_more = true` always carries `next_cursor`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct McpSearchResponse {
    #[schemars(length(max = 20))]
    pub hits: Vec<McpSearchHit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    pub has_more: bool,
}

impl McpSearchResponse {
    pub fn new(hits: Vec<McpSearchHit>, next_cursor: Option<String>) -> Self {
        let has_more = next_cursor.is_some();
        Self {
            hits,
            next_cursor,
            has_more,
        }
    }

    pub fn validate_continuation(&self) -> anyhow::Result<()> {
        if self.hits.len() > usize::from(MCP_SEARCH_LIMIT_MAX) {
            return Err(anyhow::anyhow!("hits must contain at most 20 items"));
        }
        if self.has_more && self.next_cursor.is_none() {
            return Err(anyhow::anyhow!(
                "has_more=true requires a next_cursor continuation token"
            ));
        }
        Ok(())
    }
}

/// Arguments for `get_document`: one document plus a bounded chunk window.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpDocumentArgs {
    pub document_id: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(max = 64))]
    pub locale: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(max = 256))]
    pub chunk_cursor: Option<String>,
    #[serde(default = "default_chunk_limit")]
    #[schemars(range(min = 1, max = 50))]
    pub chunk_limit: usize,
}

impl McpDocumentArgs {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.document_id <= 0 {
            return Err(anyhow::anyhow!("document_id must be a positive integer"));
        }
        if self.chunk_limit < MCP_CHUNK_LIMIT_MIN || self.chunk_limit > MCP_CHUNK_LIMIT_MAX {
            return Err(anyhow::anyhow!("chunk_limit must be between 1 and 50"));
        }
        parse_chunk_cursor(self.chunk_cursor.as_deref())?;
        if let Some(locale) = self.locale.as_deref()
            && (!non_blank(locale) || locale.chars().count() > MCP_LOCALE_MAX_CHARS)
        {
            return Err(anyhow::anyhow!("locale must be 1..=64 characters"));
        }
        Ok(())
    }

    pub fn start_offset(&self) -> anyhow::Result<usize> {
        parse_chunk_cursor(self.chunk_cursor.as_deref())
    }
}

/// Parse a chunk cursor: absent means offset 0; otherwise a non-negative
/// integer offset into the document chunk list.
pub fn parse_chunk_cursor(cursor: Option<&str>) -> anyhow::Result<usize> {
    match cursor {
        None => Ok(0),
        Some(raw) => {
            if raw.chars().count() > MCP_CURSOR_MAX_CHARS {
                return Err(anyhow::anyhow!("chunk_cursor must be 1..=256 characters"));
            }
            raw.parse::<usize>()
                .map_err(|_| anyhow::anyhow!("chunk_cursor must be a non-negative integer"))
        }
    }
}

/// Bounded document detail output with chunk continuation.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct McpDocumentDetailResponse {
    pub document: McpDocumentDetail,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_chunk_cursor: Option<String>,
    pub has_more: bool,
}

impl McpDocumentDetailResponse {
    pub fn validate_continuation(&self) -> anyhow::Result<()> {
        if self.document.chunks.len() > MCP_CHUNK_LIMIT_MAX {
            return Err(anyhow::anyhow!("chunks must contain at most 50 items"));
        }
        if self.has_more && self.next_chunk_cursor.is_none() {
            return Err(anyhow::anyhow!(
                "has_more=true requires a next_chunk_cursor continuation token"
            ));
        }
        Ok(())
    }
}

/// Build a bounded detail response for the requested chunk window.
///
/// `start` is a chunk offset, `limit` is clamped to `1..=50` by validation;
/// chunk texts are truncated to 4,000 characters to match the schema.
pub fn paginate_document_detail(
    document: &context69_contracts_search::DocumentResponse,
    start: usize,
    limit: usize,
) -> anyhow::Result<McpDocumentDetailResponse> {
    if !(MCP_CHUNK_LIMIT_MIN..=MCP_CHUNK_LIMIT_MAX).contains(&limit) {
        return Err(anyhow::anyhow!("chunk_limit must be between 1 and 50"));
    }
    let mut detail = McpDocumentDetail::header(document);
    let end = start.saturating_add(limit).min(document.chunks.len());
    let has_more = end < document.chunks.len();
    detail.chunks = document
        .chunks
        .iter()
        .skip(start)
        .take(limit)
        .map(|chunk| McpDocumentChunk {
            index: chunk.chunk_index,
            text: truncate_chars(&chunk.text, crate::projections::MCP_CHUNK_TEXT_MAX_CHARS),
        })
        .collect();
    Ok(McpDocumentDetailResponse {
        document: detail,
        next_chunk_cursor: has_more.then(|| end.to_string()),
        has_more,
    })
}

/// MCP-local structured query for `query_documents`.
///
/// Same JSON shape as the shared domain filter surface (`locale`,
/// `source_key`, published window, `metadata_filters`, `sort`, `limit`,
/// `cursor`) but owned by the MCP boundary: `limit` defaults to the MCP-local
/// 20 (never the HTTP DTO default of 50) and the schema declares the explicit
/// 1..=20 bound with cursor continuation. A typed adapter converts this into
/// the internal [`context69_contracts_search::DocumentQueryRequest`] at the service boundary.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpDocumentQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(max = 64))]
    pub locale: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(max = 256))]
    pub source_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_after: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_before: Option<DateTime<Utc>>,
    #[serde(default)]
    pub metadata_filters: Vec<context69_contracts_search::MetadataFilter>,
    #[serde(default)]
    pub sort: Vec<context69_contracts_search::DocumentSort>,
    #[serde(default = "default_mcp_query_limit")]
    #[schemars(range(min = 1, max = 20))]
    pub limit: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(max = 256))]
    pub cursor: Option<String>,
}

impl McpDocumentQuery {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.limit == 0 || self.limit > MCP_SEARCH_LIMIT_MAX {
            return Err(anyhow::anyhow!("limit must be between 1 and 20"));
        }
        if let Some(locale) = self.locale.as_deref()
            && (!non_blank(locale) || locale.chars().count() > MCP_LOCALE_MAX_CHARS)
        {
            return Err(anyhow::anyhow!("locale must be 1..=64 characters"));
        }
        if let Some(source_key) = self.source_key.as_deref()
            && (!non_blank(source_key) || source_key.chars().count() > MCP_SOURCE_KEY_MAX_CHARS)
        {
            return Err(anyhow::anyhow!("source_key must be 1..=256 characters"));
        }
        if let Some(cursor) = self.cursor.as_deref()
            && (!non_blank(cursor) || cursor.chars().count() > MCP_CURSOR_MAX_CHARS)
        {
            return Err(anyhow::anyhow!("cursor must be 1..=256 characters"));
        }
        Ok(())
    }

    /// Convert to the internal document-store query. Filter contents pass
    /// through unchanged; only the MCP window (`limit` 1..=20, opaque cursor)
    /// is normalized onto the shared DTO.
    pub fn to_document_query_request(&self) -> context69_contracts_search::DocumentQueryRequest {
        context69_contracts_search::DocumentQueryRequest {
            locale: self.locale.clone(),
            source_key: self.source_key.clone(),
            published_after: self.published_after,
            published_before: self.published_before,
            metadata_filters: self.metadata_filters.clone(),
            sort: self.sort.clone(),
            limit: usize::from(self.limit),
            cursor: self.cursor.clone(),
        }
    }
}

/// Arguments for `query_documents`: structured listing keeps its filter/sort
/// surface (that is the point of the tool) but the result window is bounded to
/// at most 20 summaries with cursor continuation.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpDocumentQueryArgs {
    #[schemars(length(min = 1, max = 1024))]
    pub group_path: String,
    pub query: McpDocumentQuery,
}

impl McpDocumentQueryArgs {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.group_path.trim().is_empty()
            || self.group_path.chars().count() > MCP_GROUP_PATH_MAX_CHARS
        {
            return Err(anyhow::anyhow!("group_path must be 1..=1024 characters"));
        }
        self.query.validate()
    }
}

/// Bounded structured-query output. `has_more = true` always carries
/// `next_cursor`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct McpDocumentQueryResponse {
    #[schemars(length(max = 20))]
    pub documents: Vec<McpDocumentSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    pub has_more: bool,
}

impl McpDocumentQueryResponse {
    pub fn new(documents: Vec<McpDocumentSummary>, next_cursor: Option<String>) -> Self {
        let has_more = next_cursor.is_some();
        Self {
            documents,
            next_cursor,
            has_more,
        }
    }

    pub fn validate_continuation(&self) -> anyhow::Result<()> {
        if self.documents.len() > usize::from(MCP_SEARCH_LIMIT_MAX) {
            return Err(anyhow::anyhow!("documents must contain at most 20 items"));
        }
        if self.has_more && self.next_cursor.is_none() {
            return Err(anyhow::anyhow!(
                "has_more=true requires a next_cursor continuation token"
            ));
        }
        Ok(())
    }
}

/// Arguments for `get_document_by_external_id`: exact key lookup plus locale.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpDocumentKeyArgs {
    #[schemars(length(min = 1, max = 1024))]
    pub group_path: String,
    pub key: context69_contracts_search::DocumentKey,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(max = 64))]
    pub locale: Option<String>,
}

impl McpDocumentKeyArgs {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.group_path.trim().is_empty()
            || self.group_path.chars().count() > MCP_GROUP_PATH_MAX_CHARS
        {
            return Err(anyhow::anyhow!("group_path must be 1..=1024 characters"));
        }
        if self.key.source_key.trim().is_empty() || self.key.external_id.trim().is_empty() {
            return Err(anyhow::anyhow!(
                "key.source_key and key.external_id must not be blank"
            ));
        }
        if let Some(locale) = self.locale.as_deref()
            && (!non_blank(locale) || locale.chars().count() > MCP_LOCALE_MAX_CHARS)
        {
            return Err(anyhow::anyhow!("locale must be 1..=64 characters"));
        }
        Ok(())
    }
}

/// MCP-local bounded batch key set for `get_documents`.
///
/// Same JSON shape as the HTTP batch request (`keys` + `locale`) but with an
/// MCP-declared `maxItems` bound: schema and runtime agree that at most 20
/// keys are accepted, and oversized lists are rejected at deserialization as
/// well as by [`McpBatchDocumentKeys::validate`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpBatchDocumentKeys {
    #[serde(deserialize_with = "deserialize_keys_max_20")]
    #[schemars(length(min = 1, max = 20))]
    pub keys: Vec<context69_contracts_search::DocumentKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(max = 64))]
    pub locale: Option<String>,
}

impl McpBatchDocumentKeys {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.keys.is_empty() || self.keys.len() > crate::projections::MCP_BATCH_KEYS_MAX {
            return Err(anyhow::anyhow!("keys must contain 1..=20 document keys"));
        }
        if let Some(locale) = self.locale.as_deref()
            && (!non_blank(locale) || locale.chars().count() > MCP_LOCALE_MAX_CHARS)
        {
            return Err(anyhow::anyhow!("locale must be 1..=64 characters"));
        }
        Ok(())
    }
}

/// Reject oversized key lists while deserializing so the schema `maxItems`
/// bound is enforced before validation runs.
fn deserialize_keys_max_20<'de, D>(
    deserializer: D,
) -> Result<Vec<context69_contracts_search::DocumentKey>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let keys = Vec::<context69_contracts_search::DocumentKey>::deserialize(deserializer)?;
    if keys.len() > crate::projections::MCP_BATCH_KEYS_MAX {
        return Err(serde::de::Error::custom(
            "keys must contain 1..=20 document keys",
        ));
    }
    Ok(keys)
}

/// Arguments for `get_documents`: bounded batch detail after search/query.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpBatchDocumentArgs {
    #[schemars(length(min = 1, max = 1024))]
    pub group_path: String,
    pub request: McpBatchDocumentKeys,
}

impl McpBatchDocumentArgs {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.group_path.trim().is_empty()
            || self.group_path.chars().count() > MCP_GROUP_PATH_MAX_CHARS
        {
            return Err(anyhow::anyhow!("group_path must be 1..=1024 characters"));
        }
        self.request.validate()
    }
}

/// One batch item: the requested key plus its bounded detail when found.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct McpBatchDocumentItem {
    pub key: context69_contracts_search::DocumentKey,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<McpDocumentDetailResponse>,
}

/// Bounded batch output. Oversized key lists are rejected up front, so
/// `has_more` is always false today; the field stays so future server-side
/// windowing cannot silently drop requested keys.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct McpBatchDocumentResponse {
    #[schemars(length(max = 20))]
    pub items: Vec<McpBatchDocumentItem>,
    pub has_more: bool,
}

/// Arguments for `list_sources`: cursor pagination over safe summaries.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpSourceListArgs {
    #[serde(default = "default_mcp_source_limit")]
    #[schemars(range(min = 1, max = 100))]
    pub limit: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(max = 256))]
    pub cursor: Option<String>,
}

impl Default for McpSourceListArgs {
    fn default() -> Self {
        Self {
            limit: MCP_SOURCE_LIMIT_DEFAULT,
            cursor: None,
        }
    }
}

impl McpSourceListArgs {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.limit < MCP_SOURCE_LIMIT_MIN || self.limit > MCP_SOURCE_LIMIT_MAX {
            return Err(anyhow::anyhow!("limit must be between 1 and 100"));
        }
        parse_offset_cursor(self.cursor.as_deref())?;
        Ok(())
    }

    pub fn start_offset(&self) -> anyhow::Result<usize> {
        parse_offset_cursor(self.cursor.as_deref())
    }
}

/// Parse an offset cursor: absent means 0; otherwise a non-negative integer.
pub fn parse_offset_cursor(cursor: Option<&str>) -> anyhow::Result<usize> {
    match cursor {
        None => Ok(0),
        Some(raw) => {
            if raw.trim().is_empty() || raw.chars().count() > MCP_CURSOR_MAX_CHARS {
                return Err(anyhow::anyhow!("cursor must be 1..=256 characters"));
            }
            raw.parse::<usize>()
                .map_err(|_| anyhow::anyhow!("cursor must be a non-negative integer offset"))
        }
    }
}

/// Safe source listing output. `has_more = true` always carries `next_cursor`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct McpSourceListResponse {
    #[schemars(length(max = 100))]
    pub sources: Vec<McpSourceSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    pub has_more: bool,
}

impl McpSourceListResponse {
    pub fn validate_continuation(&self) -> anyhow::Result<()> {
        if self.sources.len() > usize::from(MCP_SOURCE_LIMIT_MAX) {
            return Err(anyhow::anyhow!("sources must contain at most 100 items"));
        }
        if self.has_more && self.next_cursor.is_none() {
            return Err(anyhow::anyhow!(
                "has_more=true requires a next_cursor continuation token"
            ));
        }
        Ok(())
    }
}

/// Slice safe summaries into one cursor page. The cursor is an integer offset;
/// invalid cursors are rejected so callers surface a fix instead of silently
/// restarting the listing.
pub fn paginate_source_summaries(
    summaries: &[McpSourceSummary],
    args: &McpSourceListArgs,
) -> anyhow::Result<McpSourceListResponse> {
    args.validate()?;
    let start = args.start_offset()?;
    let limit = usize::from(args.limit);
    let end = start.saturating_add(limit).min(summaries.len());
    let page = if start >= summaries.len() {
        Vec::new()
    } else {
        summaries[start..end].to_vec()
    };
    let has_more = end < summaries.len();
    Ok(McpSourceListResponse {
        sources: page,
        next_cursor: has_more.then(|| end.to_string()),
        has_more,
    })
}
