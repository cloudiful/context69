use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::Visibility;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct SearchRequest {
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// DEPRECATED: use `cursor` for pagination. Kept for compatibility; it maps
    /// to an offset cursor (`(page - 1) * limit`) when no `cursor` is present.
    /// Date mode (`sort=date`) is forward-only keyset pagination: `page > 1`
    /// is rejected as a 400 validation error (the client must use the cursor
    /// returned in `next_cursor` to fetch the next page).
    #[serde(default = "default_page")]
    pub page: usize,
    #[serde(default)]
    pub source_key: Option<String>,
    #[serde(default)]
    pub group_path: Option<String>,
    #[serde(default)]
    pub published_after: Option<DateTime<Utc>>,
    #[serde(default)]
    pub published_before: Option<DateTime<Utc>>,
    /// Opaque pagination cursor returned in `SearchPagination.next_cursor` /
    /// `prev_cursor`. A cursor encodes the ordering epoch
    /// (`{rerank_applied, offset}`) of the page that issued it and is only
    /// valid within that same ordering: requests whose effective ordering
    /// differs from the cursor's epoch are rejected with a 400 instead of
    /// silently serving a differently ordered window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default)]
    pub metadata_filters: Vec<crate::MetadataFilter>,
    /// Additive ordering mode. Defaults to `relevance` so existing clients
    /// observe no change. `date` is a latest-first walk over non-overlapping
    /// `published_ts` windows without rerank; the cursor pins sort+date mode
    /// and rejects relevance cursors.
    #[serde(default = "default_sort")]
    pub sort: SearchSort,
}

fn default_limit() -> usize {
    8
}

fn default_page() -> usize {
    1
}

fn default_sort() -> SearchSort {
    SearchSort::Relevance
}

/// Ordering applied to a `SearchRequest`. `Relevance` is the default and
/// preserves the existing vector/hybrid + rerank pipeline. `Date` switches the
/// pipeline to a `published_ts DESC` walk over Qdrant that emits latest-first
/// results without rerank; the date mode requires a non-blank `query` (the
/// request is rejected as a 400 otherwise) and matches the query against
/// hydrated `title + chunk_text` with the same all-terms substring rule used
/// by the keyword path. Punctuation-only / whitespace-only queries fall back
/// to a literal phrase substring rule (the SQL equivalent of
/// `lower(title) LIKE phrase OR lower(chunk) LIKE phrase`). Same-second
/// ties follow the Qdrant `scroll` return order (deterministic per ordering
/// epoch); the cursor pins the resume key so a replayed request continues
/// the walk in the same order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SearchSort {
    Relevance,
    Date,
}

impl SearchSort {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Relevance => "relevance",
            Self::Date => "date",
        }
    }
}

impl Default for SearchSort {
    fn default() -> Self {
        Self::Relevance
    }
}

/// Windowed pagination for search responses.
///
/// Mirrors `Pagination` (page/page_size/total/has_more/total_is_exact) and
/// adds opaque `next_cursor`/`prev_cursor` values that encode the ordering
/// epoch the page was produced under. Page-based navigation stays accepted for
/// compatibility, but cursor navigation is authoritative: the same cursor is
/// only valid within one ordering (local vs reranked).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct SearchPagination {
    pub page: u32,
    pub page_size: u32,
    /// Known result count for the current window. With `total_is_exact=false`
    /// this is a lower bound, not an exact match count.
    pub total: u64,
    /// Page count derived from `total` (also a lower bound when inexact).
    pub total_pages: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_more: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_is_exact: Option<bool>,
    /// Opaque cursor for the next page in the same ordering epoch, when the
    /// service observed at least one candidate beyond this page's window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    /// Opaque cursor for the previous page in the same ordering epoch, present
    /// when this page is not the first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev_cursor: Option<String>,
}

impl SearchPagination {
    /// Build a search-window pagination with the same lower-bound semantics as
    /// `Pagination::try_new_search_window`; cursor fields are filled by the
    /// search service from the effective ordering epoch.
    pub fn try_new_search_window(
        page: u32,
        page_size: u32,
        total: u64,
        has_more: Option<bool>,
    ) -> anyhow::Result<Self> {
        if page == 0 {
            return Err(anyhow::anyhow!("page must be greater than 0"));
        }
        if !(1..=100).contains(&page_size) {
            return Err(anyhow::anyhow!("page_size must be between 1 and 100"));
        }
        let total_pages = if total == 0 {
            0
        } else {
            u32::try_from(total.div_ceil(u64::from(page_size)))?
        };
        Ok(Self {
            page,
            page_size,
            total,
            total_pages,
            has_more,
            total_is_exact: Some(false),
            next_cursor: None,
            prev_cursor: None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SearchMode {
    Vector,
    Hybrid,
}

impl SearchMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Vector => "vector",
            Self::Hybrid => "hybrid",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct SearchHit {
    pub chunk_id: Uuid,
    pub document_id: i64,
    pub group_key: String,
    pub group_path: String,
    pub visibility: Visibility,
    pub source_key: String,
    pub external_id: String,
    pub title: String,
    pub summary: Option<String>,
    pub source_uri: String,
    pub published_at: Option<DateTime<Utc>>,
    pub chunk_index: i32,
    pub chunk_text: String,
    pub score: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vector_score: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyword_score: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rerank_score: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub match_reason: Option<String>,
    #[serde(default)]
    #[schema(value_type = Object)]
    pub metadata_json: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library_file_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library_section_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library_path: Option<String>,
    #[serde(default)]
    pub is_library_file: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_locale: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_locale: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub translation_status: Option<crate::TranslationStatus>,
    #[serde(default)]
    pub is_fallback: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct SearchResponse {
    pub query: String,
    pub items: Vec<SearchHit>,
    pub pagination: SearchPagination,
}

/// One page window inside a search stream exchange. Both the `local` and the
/// `reranked` SSE events carry this shape for the same `[offset, offset+limit)`
/// window; the `reranked` payload reorders (and may adjust the membership of)
/// the page that `local` rendered first.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct SearchStreamPage {
    pub items: Vec<SearchHit>,
    pub pagination: SearchPagination,
}

/// Terminal SSE event payload: reports which ordering epoch the stream's final
/// page belongs to (`true` = reranked ordering was applied).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct SearchStreamDone {
    pub rerank_applied: bool,
}

/// Ordered frames produced by a search stream run. The transport maps each
/// variant to an SSE `event:` frame (`local`, `reranked`, `done`, `error`).
#[derive(Debug, Clone)]
pub enum SearchStreamEvent {
    Local(SearchStreamPage),
    Reranked(SearchStreamPage),
    Done(SearchStreamDone),
    Error { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct DocumentResponse {
    pub document_id: i64,
    pub group_key: String,
    pub group_path: String,
    pub visibility: Visibility,
    pub source_key: String,
    pub external_id: String,
    pub title: String,
    pub summary: Option<String>,
    pub source_uri: String,
    pub published_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    pub record_hash: String,
    #[serde(default)]
    #[schema(value_type = Object)]
    pub metadata_json: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library_file_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library_section_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library_path: Option<String>,
    #[serde(default)]
    pub is_library_file: bool,
    pub chunks: Vec<DocumentChunkResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_locale: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_locale: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub translation_status: Option<crate::TranslationStatus>,
    #[serde(default)]
    pub is_fallback: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct DocumentChunkResponse {
    pub chunk_id: Uuid,
    pub chunk_index: i32,
    pub text: String,
}
