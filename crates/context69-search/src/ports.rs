use std::collections::{HashMap, HashSet};

use anyhow::Result;
use async_trait::async_trait;
use context69_contracts_search::{DocumentResponse, SearchHit, SearchRequest};
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

/// The lifecycle phase of one search registry entry.
///
/// The registry moves `staging -> switched -> retired`, and an abandoned
/// staging run moves straight to `retired`. The phase decides which id set
/// the search paths exclude, so it is decoded once here instead of being
/// compared as a string at every call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistryPhase {
    Staging,
    Switched,
    Retired,
}

impl RegistryPhase {
    /// The phase of a stored entry, or `None` for a phase the lifecycle does
    /// not define.
    pub fn parse(phase: &str) -> Option<Self> {
        match phase {
            "staging" => Some(Self::Staging),
            "switched" => Some(Self::Switched),
            "retired" => Some(Self::Retired),
            _ => None,
        }
    }
}

/// One in-flight registry run of the future safe reingest design (issue #1138
/// P3 foundation). No production writer exists yet; the shape follows the
/// agreed staging/switch/retire lifecycle so later phases can fill it without
/// changing the read contract.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchRegistryEntry {
    pub run_id: Uuid,
    pub file_id: Option<Uuid>,
    pub document_ids: Vec<i64>,
    pub staged_ids: Vec<Uuid>,
    pub retired_ids: Vec<Uuid>,
    pub baseline_record_hash: Option<String>,
    pub staged_record_hash: Option<String>,
    pub generation: i64,
    pub phase: String,
}

impl SearchRegistryEntry {
    /// The point ids this entry keeps out of every vector candidate path.
    ///
    /// A staging run hides its staged ids, because those points are not the
    /// live version yet. A switched or retired entry hides its retired ids,
    /// which are the ids of the version that is no longer live; the staged
    /// ids of a resolved run are live again and must not be excluded.
    ///
    /// An unknown phase excludes both id lists. The database decode refuses
    /// such a phase, so this is only the defensive direction for a snapshot
    /// that never came from the database.
    pub fn excluded_point_ids(&self) -> Vec<Uuid> {
        match RegistryPhase::parse(&self.phase) {
            Some(RegistryPhase::Staging) => self.staged_ids.clone(),
            Some(RegistryPhase::Switched) | Some(RegistryPhase::Retired) => {
                self.retired_ids.clone()
            }
            None => {
                let mut ids = self.staged_ids.clone();
                ids.extend(self.retired_ids.iter().copied());
                ids
            }
        }
    }
}

/// The ids one search attempt must keep out of the vector index.
///
/// The set has three independent parts and every part is applied index-side
/// before the request limit, so an excluded point never occupies a candidate
/// slot:
///
/// * `point_ids` are the ids the registry lifecycle excludes (a staging run's
///   staged ids, a resolved run's retired ids).
/// * `file_ids` are the library files that were explicitly deleted. Their
///   points are excluded by the indexed `library_file_id` payload field, so
///   the whole file leaves the results without one registry reservation per
///   point.
/// * `document_ids` are the documents that were explicitly deleted. Their
///   points are excluded by the indexed `document_id` payload field.
///
/// An empty set is the compatible pre-existing filter: every search mode
/// composes the same exclusions, so an empty part simply adds no clause.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchExclusions {
    pub point_ids: Vec<Uuid>,
    pub file_ids: Vec<Uuid>,
    pub document_ids: Vec<i64>,
}

impl SearchExclusions {
    /// The exclusions that exclude nothing, the state of a registry without
    /// runs and without deleted entities.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Whether the set excludes nothing at all.
    pub fn is_empty(&self) -> bool {
        self.point_ids.is_empty() && self.file_ids.is_empty() && self.document_ids.is_empty()
    }
}

/// A consistent snapshot of the search registry: the epoch and the entries are
/// read in one database statement, so an empty registry is the epoch plus an
/// empty entry list and the pair can never disagree.
///
/// The deleted entity sets are part of the same statement, so a search attempt
/// never pairs an epoch with a different set of deleted files or documents.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SearchRegistryState {
    pub epoch: i64,
    /// The durable readiness of the derived original index: a non-idle rebuild
    /// barrier or a durably invalid index. A healthy mock or keyword-only state
    /// keeps the default `false`.
    pub unavailable: bool,
    pub entries: Vec<SearchRegistryEntry>,
    /// The library files that were explicitly deleted. Their points stay
    /// excluded from every search path.
    pub deleted_file_ids: Vec<Uuid>,
    /// The documents that were explicitly deleted. Their points stay excluded
    /// from every search path.
    pub deleted_document_ids: Vec<i64>,
}

impl SearchRegistryState {
    /// Reject a search before any cache hit, response or stream frame while the
    /// derived original index is unavailable.
    pub fn ensure_available(&self) -> Result<()> {
        if self.unavailable {
            return Err(context69_contracts_core::errors::DomainError::unavailable(
                "the derived search index is being rebuilt; retry after it completes",
            )
            .into());
        }
        Ok(())
    }

    /// The deduplicated point ids one search attempt must exclude from every
    /// vector candidate path. A staging run hides its staged ids; a switched
    /// or retired entry hides its retired ids, the ids of the version that is
    /// no longer live. Ids repeated across entries are kept once. An empty
    /// result means the exact pre-existing filter and is the normal state
    /// while no staging writer exists.
    pub fn search_point_exclusions(&self) -> Vec<Uuid> {
        let mut seen = HashSet::new();
        let mut point_ids = Vec::new();
        for entry in &self.entries {
            for id in entry.excluded_point_ids() {
                if seen.insert(id) {
                    point_ids.push(id);
                }
            }
        }
        point_ids
    }

    /// The complete exclusion set of one search attempt: the registry's point
    /// exclusions plus the deleted entity sets of the same snapshot.
    ///
    /// The deleted file and document ids are deduplicated, so a repeated
    /// tombstone never inflates the index-side filter.
    pub fn search_exclusions(&self) -> SearchExclusions {
        let mut file_ids = Vec::new();
        let mut seen_files = HashSet::new();
        for id in &self.deleted_file_ids {
            if seen_files.insert(*id) {
                file_ids.push(*id);
            }
        }
        let mut document_ids = Vec::new();
        let mut seen_documents = HashSet::new();
        for id in &self.deleted_document_ids {
            if seen_documents.insert(*id) {
                document_ids.push(*id);
            }
        }
        SearchExclusions {
            point_ids: self.search_point_exclusions(),
            file_ids,
            document_ids,
        }
    }
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
        exclusions: &SearchExclusions,
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

    /// Snapshot the search registry (epoch plus entries) in one database
    /// statement, taken before the reads that contribute to one search
    /// attempt. An empty registry is normal; `epoch` and `entries` always
    /// belong to the same database snapshot. Registry mutations do not exist
    /// in the search-critical path yet, so the default is a stable empty
    /// registry for tests that never exercise coherence.
    async fn read_search_registry_state(&self) -> Result<SearchRegistryState> {
        Ok(SearchRegistryState::default())
    }

    /// The current registry epoch from one scalar read. A search re-checks it
    /// after every read contributing to its output and discards the attempt
    /// when it moved. See [`SearchRepository::read_search_registry_state`].
    async fn read_search_registry_epoch(&self) -> Result<i64> {
        Ok(0)
    }
}

#[async_trait]
pub trait SearchEmbeddingProvider: Send + Sync {
    async fn embed_query(&self, query: &str) -> Result<Vec<f32>>;

    /// A key identifying the vector space the current provider produces, used to
    /// scope cached query vectors. It must change whenever the live provider or
    /// its identity changes, so a cached vector is never reused across two
    /// different embedding identities. An empty value asks the caller to fall
    /// back to its configured startup identity.
    fn embedding_identity(&self) -> String {
        String::new()
    }
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
    /// paths; date mode bypasses this with `search_by_date_window`. The
    /// `exclusions` must be applied as index-side `must_not` filters before
    /// the request limit so excluded points never occupy a candidate slot; an
    /// empty set is the pre-existing filter.
    async fn search(
        &self,
        vector: Vec<f32>,
        request: &SearchRequest,
        scope: &AccessScope,
        exclusions: &SearchExclusions,
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
        exclusions: &SearchExclusions,
    ) -> Result<DateWindowPage>;
}
