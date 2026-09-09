use std::collections::HashMap;

use anyhow::Result;
use async_trait::async_trait;
use context69_contracts::{DocumentResponse, SearchHit, SearchRequest};
use uuid::Uuid;

use crate::{AccessScope, SearchPointHit, SearchSettings, StoredRerankItemScore};

#[async_trait]
pub trait SearchScopeResolver: Send + Sync {
    async fn access_scope(
        &self,
        user_id: Option<i64>,
        group_path: Option<String>,
    ) -> Result<AccessScope>;
}

#[async_trait]
pub trait SearchRepository: Send + Sync {
    async fn get_search_settings(&self) -> Result<Option<SearchSettings>>;
    async fn get_search_generation(&self) -> Result<i64>;
    /// Snapshot the maximum `published_ts` value visible to the caller under
    /// the request's filter set. Used by the date-mode pipeline as the
    /// wall-clock upper bound for the first window so a replayed cursor
    /// cannot silently pick up freshly indexed records. The `user_id` is
    /// the caller's identity (same as the surrounding search request) so
    /// the snapshot honours the user's visibility constraints; passing
    /// `None` for a logged-in caller would otherwise exclude the user's
    /// own private records from the ceiling and miss their newest rows on
    /// page 1.
    ///
    /// Returns `Ok(None)` only when the index has no visible rows under the
    /// caller's scope. A Qdrant / repository failure must propagate as
    /// `Err(_)` so the date pipeline can distinguish "empty index" from
    /// "snapshot fetch failed" and surface the failure as a 500 instead of
    /// silently widening the replayed upper bound.
    async fn date_request_upper_bound(
        &self,
        user_id: Option<i64>,
        request: &SearchRequest,
    ) -> Result<Option<i64>>;
    async fn fetch_search_hits_by_chunk_ids(
        &self,
        chunk_ids: &[Uuid],
        request: &SearchRequest,
        scope: &AccessScope,
    ) -> Result<HashMap<Uuid, SearchHit>>;
    async fn keyword_search(
        &self,
        request: &SearchRequest,
        scope: &AccessScope,
        limit: usize,
    ) -> Result<Vec<SearchHit>>;
    async fn list_rerank_item_scores(
        &self,
        rerank_model: &str,
        query_hash: &str,
        chunk_ids: &[Uuid],
    ) -> Result<HashMap<Uuid, StoredRerankItemScore>>;
    async fn upsert_rerank_item_scores(&self, scores: &[StoredRerankItemScore]) -> Result<()>;
    async fn get_document(
        &self,
        document_id: i64,
        scope: &AccessScope,
    ) -> Result<Option<DocumentResponse>>;
}

#[async_trait]
pub trait SearchEmbeddingProvider: Send + Sync {
    async fn embed_query(&self, query: &str) -> Result<Vec<f32>>;
}

/// Result of a single date-mode window query against the vector index.
///
/// `chunk_id` is the deduplication key; the `published_ts` and `score` come
/// straight from the Qdrant payload. `score` carries the vector similarity
/// used only for hydration, not for sort order (the date pipeline sorts by
/// `published_ts` after hydration).
#[derive(Debug, Clone)]
pub struct SearchDatePointHit {
    pub chunk_id: Uuid,
    pub published_ts: Option<i64>,
    pub score: f32,
}

/// Lower bound for a `search_by_date_window` call.
///
/// The bound is always exclusive so consecutive windows never overlap: when a
/// window completes with an oldest `published_ts = T`, the next window asks
/// for `published_ts <= T - 1` and `published_ts > lower`. Same-second
/// records at the boundary are revisited by no more than one window and
/// deduplicated by `chunk_id` in the pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateBound {
    /// Inclusive bound: `published_ts <= value`.
    Inclusive(i64),
    /// Bound is open (no constraint).
    Unbounded,
}

/// Arguments for `SearchIndex::search_by_date_window`. Grouped to keep
/// the port signature bounded and to give the keyset walk a single,
/// auditable state struct.
#[derive(Debug, Clone, Copy)]
pub struct DateWindowQuery {
    /// Exclusive lower bound (`published_ts > after`).
    pub after: DateBound,
    /// Inclusive upper bound (`published_ts <= before`).
    pub before: DateBound,
    /// Index-side fetch limit.
    pub limit: usize,
    /// Qdrant-side keyset cursor for paging within the same
    /// `published_ts` value. `None` means "start from the first
    /// point in the window".
    pub offset: Option<uuid::Uuid>,
}

/// One page of a date-mode window fetch plus the Qdrant-side offset for
/// resuming the same window (in-timestamp pagination via `next_page_offset`).
/// `next_offset = None` means the window is exhausted; the pipeline may
/// advance the `before` bound to the next older timestamp. The pipeline
/// threads the offset back into the next call (and across requests via the
/// `c4:` cursor) so the Qdrant `scroll` return order is the authoritative
/// ordering — the pipeline does NOT re-sort in memory and the in-memory
/// `seen` set is the only dedup mechanism.
#[derive(Debug, Clone)]
pub struct DateWindowPage {
    pub hits: Vec<SearchDatePointHit>,
    /// Opaque Qdrant-side offset for the next page within the same
    /// `published_ts` value. `None` when this page is the tail of the
    /// current window. The date pipeline threads this into the next
    /// `search_by_date_window` call (and into the `c4:` cursor for
    /// cross-request resume) so a replayed request resumes the
    /// keyset walk exactly after the last emitted record.
    pub next_offset: Option<uuid::Uuid>,
}

#[async_trait]
pub trait SearchIndex: Send + Sync {
    /// Relevance-ordering vector search. Used by the default and `sort=relevance`
    /// paths; date mode bypasses this with `search_by_date_window`.
    async fn search(
        &self,
        vector: Vec<f32>,
        request: &SearchRequest,
        scope: &AccessScope,
    ) -> Result<Vec<SearchPointHit>>;

    /// Latest-first date window: returns up to `limit` points ordered by
    /// `published_ts DESC`. The Qdrant `scroll` return order is the
    /// authoritative ordering inside a same-second boundary — the
    /// pipeline does NOT re-sort in memory. `offset` pages within the
    /// same `published_ts` value (drain the entire boundary before
    /// advancing to an older timestamp) and is the resume key for
    /// both in-request and cross-request pagination. The returned
    /// `next_offset` is `Some(uuid)` when the Qdrant side knows more
    /// points satisfy the filter so the pipeline can keep paging
    /// without losing records; the pipeline additionally bounds the
    /// per-request walk with `MAX_DATE_WINDOWS` so a single request
    /// cannot run unbounded.
    async fn search_by_date_window(
        &self,
        vector: Vec<f32>,
        request: &SearchRequest,
        scope: &AccessScope,
        query: DateWindowQuery,
    ) -> Result<DateWindowPage>;
}
