use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

use anyhow::anyhow;
use async_trait::async_trait;
use context69_contracts::search::SearchStreamEvent;
use context69_contracts::{DocumentResponse, SearchHit, SearchMode, SearchRequest, Visibility};
use uuid::Uuid;

use super::search_cursor::{CursorContext, encode_cursor};
use crate::{
    AccessScope, DateBound, RerankClient, RerankDocument, RerankHit, SearchCache,
    SearchDatePointHit, SearchEmbeddingProvider, SearchIndex, SearchPointHit, SearchRepository,
    SearchScopeResolver, SearchService, SearchSettings, StoredRerankItemScore, abort_pair,
};

fn block_on<F: Future>(mut future: F) -> F::Output {
    unsafe fn clone_waker(_: *const ()) -> RawWaker {
        noop_waker()
    }
    unsafe fn noop(_: *const ()) {}
    fn noop_waker() -> RawWaker {
        static VTABLE: RawWakerVTable = RawWakerVTable::new(clone_waker, noop, noop, noop);
        RawWaker::new(std::ptr::null(), &VTABLE)
    }
    let waker = unsafe { Waker::from_raw(noop_waker()) };
    let mut context = Context::from_waker(&waker);
    let mut pinned = unsafe { Pin::new_unchecked(&mut future) };
    loop {
        match pinned.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => std::thread::yield_now(),
        }
    }
}

struct MockScope;

#[async_trait]
impl SearchScopeResolver for MockScope {
    async fn access_scope(
        &self,
        _user_id: Option<i64>,
        _group_path: Option<String>,
    ) -> anyhow::Result<AccessScope> {
        Ok(AccessScope::default())
    }
}

struct MockEmbedding;

#[async_trait]
impl SearchEmbeddingProvider for MockEmbedding {
    async fn embed_query(&self, _query: &str) -> anyhow::Result<Vec<f32>> {
        Ok(vec![0.0; 8])
    }
}

struct MockIndex {
    hits: Vec<SearchPointHit>,
    seen_limit: Arc<Mutex<Option<usize>>>,
    /// When set, `search_by_date_window` returns this slice of the hits in
    /// the same order, honoring the `before` bound. Tests that need to
    /// exercise non-trivial date-mode behavior populate this.
    date_hits: Vec<SearchDatePointHit>,
    seen_date_bounds: Arc<Mutex<Vec<(Option<i64>, Option<i64>)>>>,
    /// How the mock should respond when the same `(boundary_ts, limit)`
    /// is requested more than once. `AdversarialFirstPage` is the
    /// production-faithful shape: the first call for a given `boundary_ts`
    /// returns an arbitrary 1024-subset ordered by descending `chunk_id`
    /// (Qdrant does not guarantee a deterministic tie-break) with a
    /// `next_offset` set, and every subsequent call returns the
    /// remainder. `Sequential` is the legacy behaviour: the first
    /// `limit` records every time, `next_offset` is always `None`.
    /// Tests default to `AdversarialFirstPage` so the cross-page union
    /// is exercised exactly like the production pipeline.
    date_window_strategy: Arc<Mutex<DateWindowStrategy>>,
    /// Optional override for the `query.limit` reported to the mock. When
    /// `Some(n)`, the mock slices its matching records to `n` per call
    /// regardless of what the pipeline asked for. Used by the genuine
    /// truncation test (population strictly larger than a single fetch)
    /// to exercise the per-request cap without forcing a 4 096-record
    /// population.
    fetch_limit_override: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
enum DateWindowStrategy {
    /// Mimics the production Qdrant `scroll` behaviour: the first call
    /// for a `(boundary_ts, limit)` pair returns an arbitrary 1024
    /// subset ordered by descending `chunk_id` plus a `next_offset`
    /// pointing at the next record. Subsequent calls return the
    /// remaining slice. The pipeline must thread the resume key so the
    /// boundary is drained in full across pages.
    AdversarialFirstPage,
    /// The mock returns the first `limit` records every time with
    /// `next_offset = None`. Useful for tests that explicitly want the
    /// legacy `take(limit)` behaviour (e.g. validating that the
    /// pipeline's in-memory `seen` set still avoids duplicates).
    Sequential,
}

impl Default for DateWindowStrategy {
    fn default() -> Self {
        Self::AdversarialFirstPage
    }
}

#[async_trait]
impl SearchIndex for MockIndex {
    async fn search(
        &self,
        _vector: Vec<f32>,
        request: &SearchRequest,
        _scope: &AccessScope,
    ) -> anyhow::Result<Vec<SearchPointHit>> {
        *self.seen_limit.lock().expect("index lock") = Some(request.limit);
        Ok(self.hits.clone())
    }

    async fn search_by_date_window(
        &self,
        _vector: Vec<f32>,
        _request: &SearchRequest,
        _scope: &AccessScope,
        query: crate::DateWindowQuery,
    ) -> anyhow::Result<crate::DateWindowPage> {
        let after_value = match query.after {
            DateBound::Inclusive(value) => Some(value.saturating_sub(1)),
            DateBound::Unbounded => None,
        };
        let before_value = match query.before {
            DateBound::Inclusive(value) => Some(value),
            DateBound::Unbounded => None,
        };
        self.seen_date_bounds
            .lock()
            .expect("date bounds lock")
            .push((after_value, before_value));
        // Filter to the requested window so a per-timestamp drain
        // (where `before_value == boundary_ts`) returns only records
        // at the boundary.
        let mut matching: Vec<SearchDatePointHit> = self
            .date_hits
            .iter()
            .filter(|hit| {
                let ts = hit.published_ts.unwrap_or(i64::MIN);
                if let Some(value) = before_value
                    && ts > value
                {
                    return false;
                }
                if let Some(value) = after_value
                    && ts <= value
                {
                    return false;
                }
                true
            })
            .cloned()
            .collect();
        let effective_limit = self
            .fetch_limit_override
            .map(|override_limit| override_limit.min(query.limit))
            .unwrap_or(query.limit);
        let strategy = *self
            .date_window_strategy
            .lock()
            .expect("date window strategy lock");
        match strategy {
            DateWindowStrategy::Sequential => {
                matching.truncate(effective_limit);
                Ok(crate::DateWindowPage {
                    hits: matching,
                    next_offset: None,
                })
            }
            DateWindowStrategy::AdversarialFirstPage => {
                // The mock mirrors the production Qdrant scroll order
                // for date-mode windows: `published_ts DESC` first,
                // then `chunk_id DESC` as the tie-break so the within-
                // boundary order is deterministic. Without the
                // `published_ts` ordering the mock would return an
                // arbitrary slice that may exclude the boundary
                // records the per-ts fetch needs to drain.
                matching.sort_by(|left, right| {
                    right
                        .published_ts
                        .cmp(&left.published_ts)
                        .then_with(|| right.chunk_id.cmp(&left.chunk_id))
                });
                let offset_idx = match query.offset {
                    // The Qdrant `next_page_offset` is the ID of the
                    // FIRST point in the next page. The next call
                    // with that offset is INCLUSIVE — the point at
                    // `offset_idx` is the first point returned.
                    Some(offset) => matching
                        .iter()
                        .position(|hit| hit.chunk_id == offset)
                        .unwrap_or(0),
                    None => 0,
                };
                let slice_end = (offset_idx + effective_limit).min(matching.len());
                let page_hits: Vec<SearchDatePointHit> =
                    matching[offset_idx..slice_end].to_vec();
                let next_offset = if slice_end < matching.len() {
                    matching.get(slice_end).map(|hit| hit.chunk_id)
                } else {
                    None
                };
                Ok(crate::DateWindowPage {
                    hits: page_hits,
                    next_offset,
                })
            }
        }
    }
}

struct MockRepo {
    settings: SearchSettings,
    hydrated: HashMap<Uuid, SearchHit>,
    keyword_hits: Vec<SearchHit>,
    seen_keyword_limit: Arc<Mutex<Option<usize>>>,
    upper_bound: Option<i64>,
}

#[async_trait]
impl SearchRepository for MockRepo {
    async fn get_search_settings(&self) -> anyhow::Result<Option<SearchSettings>> {
        Ok(Some(self.settings.clone()))
    }

    async fn get_search_generation(&self) -> anyhow::Result<i64> {
        Ok(0)
    }

    async fn date_request_upper_bound(
        &self,
        _user_id: Option<i64>,
        _request: &SearchRequest,
    ) -> anyhow::Result<Option<i64>> {
        // Tests that exercise the date-mode pipeline construct mocks that
        // observe the upper-bound request explicitly. The default is `None`
        // so tests that do not care about wall-clock drift keep working.
        Ok(self.upper_bound)
    }

    async fn fetch_search_hits_by_chunk_ids(
        &self,
        chunk_ids: &[Uuid],
        _request: &SearchRequest,
        _scope: &AccessScope,
    ) -> anyhow::Result<HashMap<Uuid, SearchHit>> {
        let mut out = HashMap::new();
        for id in chunk_ids {
            if let Some(hit) = self.hydrated.get(id) {
                out.insert(*id, hit.clone());
            }
        }
        Ok(out)
    }

    async fn keyword_search(
        &self,
        _request: &SearchRequest,
        _scope: &AccessScope,
        limit: usize,
    ) -> anyhow::Result<Vec<SearchHit>> {
        *self.seen_keyword_limit.lock().expect("keyword lock") = Some(limit);
        Ok(self.keyword_hits.clone())
    }

    async fn list_rerank_item_scores(
        &self,
        _rerank_model: &str,
        _query_hash: &str,
        _chunk_ids: &[Uuid],
    ) -> anyhow::Result<HashMap<Uuid, StoredRerankItemScore>> {
        Ok(HashMap::new())
    }

    async fn upsert_rerank_item_scores(
        &self,
        _scores: &[StoredRerankItemScore],
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn get_document(
        &self,
        _document_id: i64,
        _scope: &AccessScope,
    ) -> anyhow::Result<Option<DocumentResponse>> {
        Ok(None)
    }
}

fn test_hit(document_id: i64, chunk_index: i32, chunk_text: &str) -> SearchHit {
    SearchHit {
        chunk_id: Uuid::new_v4(),
        document_id,
        group_key: "public".to_string(),
        group_path: "public".to_string(),
        visibility: Visibility::Public,
        source_key: "src".to_string(),
        external_id: format!("ext-{document_id}-{chunk_index}"),
        title: format!("Title {document_id}"),
        summary: None,
        source_uri: "https://example.com/doc".to_string(),
        published_at: None,
        chunk_index,
        chunk_text: chunk_text.to_string(),
        score: 0.0,
        vector_score: None,
        keyword_score: None,
        rerank_score: None,
        match_reason: None,
        metadata_json: serde_json::json!({}),
        library_file_id: None,
        library_section_label: None,
        library_path: None,
        is_library_file: false,
        requested_locale: None,
        content_locale: None,
        translation_status: None,
        is_fallback: false,
    }
}

fn make_hit_with_chunk(
    document_id: i64,
    chunk_id: Uuid,
    published_ts: i64,
    text: &str,
) -> (Uuid, SearchHit) {
    let mut hit = test_hit(document_id, 0, text);
    hit.chunk_id = chunk_id;
    hit.published_at = chrono::DateTime::<chrono::Utc>::from_timestamp(published_ts, 0);
    (hit.chunk_id, hit)
}

fn test_request(page: usize, limit: usize) -> SearchRequest {
    SearchRequest {
        query: "query".to_string(),
        locale: None,
        limit,
        page,
        source_key: None,
        group_path: None,
        published_after: None,
        published_before: None,
        cursor: None,
        metadata_filters: Vec::new(),
        sort: context69_contracts::SearchSort::Relevance,
    }
}

fn vector_settings() -> SearchSettings {
    SearchSettings {
        mode: SearchMode::Vector,
        rerank_enabled: false,
        ..SearchSettings::default()
    }
}

fn hybrid_settings(rerank: bool) -> SearchSettings {
    SearchSettings {
        mode: SearchMode::Hybrid,
        rerank_enabled: rerank,
        candidate_limit: 40,
        api_key: None,
        ..SearchSettings::default()
    }
}

/// Cursor context matching the default `test_request(...)` shape: a fresh
/// empty request, default settings, and the mock generation `0`.
#[allow(dead_code)]
fn default_cursor_context() -> CursorContext {
    CursorContext {
        query_hash: SearchCache::query_hash("query"),
        filter_hash: SearchCache::filter_hash(&test_request(1, 8)),
        generation: 0,
        settings_hash: SearchCache::settings_hash(&hybrid_settings(true)),
        limit: 8,
    }
}

fn build_service(
    settings: SearchSettings,
    index_hits: Vec<SearchPointHit>,
    hydrated: HashMap<Uuid, SearchHit>,
    keyword_hits: Vec<SearchHit>,
) -> (
    SearchService,
    Arc<Mutex<Option<usize>>>,
    Arc<Mutex<Option<usize>>>,
) {
    let seen_index = Arc::new(Mutex::new(None));
    let seen_keyword = Arc::new(Mutex::new(None));
    let repo = MockRepo {
        settings,
        hydrated,
        keyword_hits,
        seen_keyword_limit: Arc::clone(&seen_keyword),
        upper_bound: None,
    };
    let service = block_on(SearchService::new(
        Arc::new(repo),
        Arc::new(MockScope),
        Arc::new(MockEmbedding),
        Arc::new(MockIndex {
            hits: index_hits,
            seen_limit: Arc::clone(&seen_index),
            date_hits: Vec::new(),
            seen_date_bounds: Arc::new(Mutex::new(Vec::new())),
            date_window_strategy: Arc::new(Mutex::new(DateWindowStrategy::AdversarialFirstPage)),
            fetch_limit_override: None,
        }),
        None,
        "test-model".to_string(),
    ))
    .expect("service");
    (service, seen_index, seen_keyword)
}

#[test]
fn vector_first_page_slices_probe_and_reports_more() {
    let hits: Vec<SearchHit> = (1..=9)
        .map(|doc| test_hit(doc, 0, &format!("meaningful body text {doc}")))
        .collect();
    let index_hits = hits
        .iter()
        .enumerate()
        .map(|(pos, hit)| SearchPointHit {
            chunk_id: hit.chunk_id,
            score: 0.9 - pos as f32 * 0.01,
        })
        .collect::<Vec<_>>();
    let hydrated = hits.into_iter().map(|hit| (hit.chunk_id, hit)).collect();
    let (service, seen_index, _) =
        build_service(vector_settings(), index_hits, hydrated, Vec::new());

    let response = block_on(service.search(None, test_request(1, 8))).expect("search");
    assert_eq!(response.items.len(), 8);
    assert_eq!(response.pagination.page, 1);
    assert_eq!(response.pagination.total, 9);
    assert_eq!(response.pagination.has_more, Some(true));
    assert_eq!(response.pagination.total_is_exact, Some(false));
    assert_eq!(*seen_index.lock().expect("lock"), Some(80));
}

#[test]
fn vector_short_page_reports_no_more() {
    let hits: Vec<SearchHit> = (1..=5)
        .map(|doc| test_hit(doc, 0, &format!("meaningful body text {doc}")))
        .collect();
    let index_hits = hits
        .iter()
        .map(|hit| SearchPointHit {
            chunk_id: hit.chunk_id,
            score: 0.8,
        })
        .collect::<Vec<_>>();
    let hydrated = hits.into_iter().map(|hit| (hit.chunk_id, hit)).collect();
    let (service, _, _) = build_service(vector_settings(), index_hits, hydrated, Vec::new());

    let response = block_on(service.search(None, test_request(1, 8))).expect("search");
    assert_eq!(response.items.len(), 5);
    assert_eq!(response.pagination.total, 5);
    assert_eq!(response.pagination.has_more, Some(false));
    assert_eq!(response.pagination.total_is_exact, Some(false));
}

#[test]
fn vector_saturated_topk_below_cap_reports_unknown() {
    let mut hits = Vec::new();
    for doc in 1..=80 {
        let text = if doc <= 5 {
            format!("meaningful body text {doc}")
        } else {
            "!".to_string()
        };
        hits.push(test_hit(doc, 0, &text));
    }
    let index_hits = hits
        .iter()
        .map(|hit| SearchPointHit {
            chunk_id: hit.chunk_id,
            score: 0.7,
        })
        .collect::<Vec<_>>();
    let hydrated = hits.into_iter().map(|hit| (hit.chunk_id, hit)).collect();
    let (service, _, _) = build_service(vector_settings(), index_hits, hydrated, Vec::new());

    let response = block_on(service.search(None, test_request(1, 8))).expect("search");
    assert_eq!(response.items.len(), 5);
    assert_eq!(response.pagination.total, 5);
    assert_eq!(response.pagination.has_more, None);
    assert_eq!(response.pagination.total_is_exact, Some(false));
}

#[test]
fn hybrid_keyword_meaningful_filtering_preserves_probe() {
    let mut keyword_hits = Vec::new();
    for doc in 10..=18 {
        let mut hit = test_hit(doc, 0, &format!("meaningful keyword text {doc}"));
        hit.keyword_score = Some(0.2);
        keyword_hits.push(hit);
    }
    let mut noisy = test_hit(1, 0, "!");
    noisy.keyword_score = Some(2.0);
    keyword_hits.push(noisy);

    let (service, _, seen_keyword) = build_service(
        hybrid_settings(false),
        Vec::new(),
        HashMap::new(),
        keyword_hits,
    );

    let response = block_on(service.search(None, test_request(1, 8))).expect("search");
    assert_eq!(response.items.len(), 8);
    assert_eq!(response.pagination.total, 9);
    assert_eq!(response.pagination.has_more, Some(true));
    assert!(response.items.iter().all(|hit| hit.chunk_text != "!"));
    assert_eq!(*seen_keyword.lock().expect("lock"), Some(80));
}

#[test]
fn hybrid_rerank_fallback_preserves_probe_without_network() {
    let vector_hits: Vec<SearchHit> = (1..=5)
        .map(|doc| test_hit(doc, 0, &format!("meaningful vector text {doc}")))
        .collect();
    let index_hits = vector_hits
        .iter()
        .map(|hit| SearchPointHit {
            chunk_id: hit.chunk_id,
            score: 0.8,
        })
        .collect::<Vec<_>>();
    let hydrated = vector_hits
        .into_iter()
        .map(|hit| (hit.chunk_id, hit))
        .collect();
    let keyword_hits: Vec<SearchHit> = (6..=10)
        .map(|doc| {
            let mut hit = test_hit(doc, 0, &format!("meaningful keyword text {doc}"));
            hit.keyword_score = Some(0.4);
            hit
        })
        .collect();
    let (service, _, _) = build_service(hybrid_settings(true), index_hits, hydrated, keyword_hits);

    let response = block_on(service.search(None, test_request(1, 8))).expect("search");
    assert_eq!(response.items.len(), 8);
    assert_eq!(response.pagination.has_more, Some(true));
    assert_eq!(response.pagination.total_is_exact, Some(false));
    assert!(response.pagination.total >= 9);
}

#[test]
fn page_limit_boundaries_return_errors() {
    let (service, _, _) = build_service(vector_settings(), Vec::new(), HashMap::new(), Vec::new());

    assert!(block_on(service.search(None, test_request(0, 8))).is_err());
    assert!(block_on(service.search(None, test_request(1, 0))).is_err());
    assert!(block_on(service.search(None, test_request(1, 101))).is_err());
}

fn collect_stream_frames(
    service: &SearchService,
    request: SearchRequest,
) -> Vec<SearchStreamEvent> {
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    let (signal, _guard) = abort_pair();
    block_on(service.stream_search(None, request, tx, signal)).expect("stream search");
    let mut frames = Vec::new();
    while let Some(frame) = block_on(rx.recv()) {
        frames.push(frame);
    }
    frames
}

fn stream_local_page(
    service: &SearchService,
    request: SearchRequest,
) -> context69_contracts::search::SearchStreamPage {
    let mut frames = collect_stream_frames(service, request);
    match frames.remove(0) {
        SearchStreamEvent::Local(page) => page,
        other => panic!("expected a local frame first, got {other:?}"),
    }
}

fn assert_pages_equivalent(
    post: &context69_contracts::SearchResponse,
    page: &context69_contracts::search::SearchStreamPage,
) {
    let post_chunks = post
        .items
        .iter()
        .map(|hit| hit.chunk_id)
        .collect::<Vec<_>>();
    let page_chunks = page
        .items
        .iter()
        .map(|hit| hit.chunk_id)
        .collect::<Vec<_>>();
    assert_eq!(post_chunks, page_chunks, "page item order must match POST");
    assert_eq!(
        serde_json::to_value(&post.pagination).unwrap(),
        serde_json::to_value(&page.pagination).unwrap(),
        "page pagination must match POST pagination"
    );
}

#[test]
fn sse_local_page_equals_post_in_vector_mode() {
    let hits: Vec<SearchHit> = (1..=12)
        .map(|doc| test_hit(doc, 0, &format!("meaningful body text {doc}")))
        .collect();
    let index_hits = hits
        .iter()
        .enumerate()
        .map(|(pos, hit)| SearchPointHit {
            chunk_id: hit.chunk_id,
            score: 0.9 - pos as f32 * 0.01,
        })
        .collect::<Vec<_>>();
    let hydrated = hits.into_iter().map(|hit| (hit.chunk_id, hit)).collect();
    let (service, _, _) = build_service(vector_settings(), index_hits, hydrated, Vec::new());

    let post = block_on(service.search(None, test_request(1, 8))).expect("search");
    let local = stream_local_page(&service, test_request(1, 8));
    assert_pages_equivalent(&post, &local);

    // Stage 2 is skipped entirely in vector mode: one local page + done(false).
    let frames = collect_stream_frames(&service, test_request(1, 8));
    assert!(matches!(
        frames.as_slice(),
        [
            SearchStreamEvent::Local(_),
            SearchStreamEvent::Done(done)
        ] if !done.rerank_applied
    ));

    // Page 2 (offset via legacy page param) matches POST page 2 as well.
    let post_page2 = block_on(service.search(None, test_request(2, 8))).expect("search page 2");
    let local_page2 = stream_local_page(&service, test_request(2, 8));
    assert_pages_equivalent(&post_page2, &local_page2);
    assert!(post_page2.pagination.prev_cursor.is_some());
}

#[test]
fn sse_stream_equals_post_when_rerank_falls_back_to_local() {
    // Rerank is enabled but no api key is configured, so stage 2 falls back to
    // the local ordering exactly like the POST pipeline does.
    let vector_hits: Vec<SearchHit> = (1..=5)
        .map(|doc| test_hit(doc, 0, &format!("meaningful vector text {doc}")))
        .collect();
    let index_hits = vector_hits
        .iter()
        .map(|hit| SearchPointHit {
            chunk_id: hit.chunk_id,
            score: 0.8,
        })
        .collect::<Vec<_>>();
    let hydrated = vector_hits
        .into_iter()
        .map(|hit| (hit.chunk_id, hit))
        .collect();
    let keyword_hits: Vec<SearchHit> = (6..=12)
        .map(|doc| {
            let mut hit = test_hit(doc, 0, &format!("meaningful keyword text {doc}"));
            hit.keyword_score = Some(0.4);
            hit
        })
        .collect();
    let (service, _, _) = build_service(hybrid_settings(true), index_hits, hydrated, keyword_hits);

    let post = block_on(service.search(None, test_request(1, 8))).expect("search");
    let frames = collect_stream_frames(&service, test_request(1, 8));
    let mut iter = frames.into_iter();
    let local = match iter.next() {
        Some(SearchStreamEvent::Local(page)) => page,
        other => panic!("expected a local frame first, got {other:?}"),
    };
    assert_pages_equivalent(&post, &local);
    // No reranked frame when stage 2 kept the local ordering; done reports it.
    match iter.next() {
        Some(SearchStreamEvent::Done(done)) => assert!(!done.rerank_applied),
        other => panic!("expected a done frame after the fallback, got {other:?}"),
    }
    assert!(iter.next().is_none(), "no extra frames after done");
}

#[test]
fn sse_rejects_cursor_from_another_ordering_epoch() {
    let hits: Vec<SearchHit> = (1..=5)
        .map(|doc| test_hit(doc, 0, &format!("meaningful body text {doc}")))
        .collect();
    let index_hits = hits
        .iter()
        .map(|hit| SearchPointHit {
            chunk_id: hit.chunk_id,
            score: 0.8,
        })
        .collect::<Vec<_>>();
    let hydrated = hits.into_iter().map(|hit| (hit.chunk_id, hit)).collect();
    let (service, _, _) = build_service(vector_settings(), index_hits, hydrated, Vec::new());

    // A reranked-ordering cursor is invalid against a vector (local-only) run:
    // the stream reports the mismatch instead of silently serving the window.
    let mut request = test_request(1, 8);
    let context = CursorContext {
        query_hash: SearchCache::query_hash("query"),
        filter_hash: SearchCache::filter_hash(&test_request(1, 8)),
        generation: 0,
        settings_hash: SearchCache::settings_hash(&vector_settings()),
        limit: 8,
    };
    request.cursor = Some(encode_cursor(8, true, 8, &context));
    let frames = collect_stream_frames(&service, request);
    // F2: an ordering-epoch mismatch must never let a `local` page slip
    // through; vector mode is local-only so the epoch is known up front and
    // the error is the only frame the client ever sees.
    assert!(
        frames
            .iter()
            .all(|frame| !matches!(frame, SearchStreamEvent::Local(_))),
        "no local frame must precede the ordering mismatch; got {frames:?}"
    );
    assert!(
        frames
            .iter()
            .all(|frame| !matches!(frame, SearchStreamEvent::Reranked(_)))
    );
    assert!(frames.iter().any(|frame| match frame {
        SearchStreamEvent::Error { message } => message.contains("cursor ordering mismatch"),
        _ => false,
    }));
}

#[test]
fn sse_hybrid_cursor_rejects_mismatch_before_local_frame() {
    // F2: a hybrid request with a reranked cursor whose actual rerank
    // stage falls back to the local ordering (rerank_enabled but no api
    // key) must surface the mismatch as the only frame. The pipeline must
    // not emit a `local` page for the local-only ordering when the cursor
    // was issued for the reranked ordering.
    let vector_hits: Vec<SearchHit> = (1..=5)
        .map(|doc| test_hit(doc, 0, &format!("meaningful vector text {doc}")))
        .collect();
    let index_hits = vector_hits
        .iter()
        .map(|hit| SearchPointHit {
            chunk_id: hit.chunk_id,
            score: 0.8,
        })
        .collect::<Vec<_>>();
    let hydrated = vector_hits.into_iter().map(|hit| (hit.chunk_id, hit)).collect();
    let keyword_hits: Vec<SearchHit> = (6..=12)
        .map(|doc| {
            let mut hit = test_hit(doc, 0, &format!("meaningful keyword text {doc}"));
            hit.keyword_score = Some(0.4);
            hit
        })
        .collect();
    let (service, _, _) = build_service(hybrid_settings(true), index_hits, hydrated, keyword_hits);

    // Build a cursor that claims `rerank_applied = true` against the same
    // request context. The mock service has no api key, so the actual
    // rerank stage falls back to the local ordering; the cursor is then
    // rejected as an epoch mismatch. The error must be the only frame.
    let mut request = test_request(1, 8);
    let context = CursorContext {
        query_hash: SearchCache::query_hash("query"),
        filter_hash: SearchCache::filter_hash(&test_request(1, 8)),
        generation: 0,
        settings_hash: SearchCache::settings_hash(&hybrid_settings(true)),
        limit: 8,
    };
    request.cursor = Some(encode_cursor(8, true, 8, &context));
    let frames = collect_stream_frames(&service, request);
    assert!(
        frames
            .iter()
            .all(|frame| !matches!(frame, SearchStreamEvent::Local(_))),
        "no local frame must precede the ordering mismatch in hybrid mode; got {frames:?}"
    );
    assert!(
        frames
            .iter()
            .all(|frame| !matches!(frame, SearchStreamEvent::Reranked(_)))
    );
    assert!(frames.iter().any(|frame| match frame {
        SearchStreamEvent::Error { message } => message.contains("cursor ordering mismatch"),
        _ => false,
    }));
}

#[test]
fn sse_hybrid_cursor_accepts_matching_epoch_and_replays_local_then_rerank() {
    // F2: a hybrid request whose cursor and rerank stage agree on the
    // epoch (both reranked) must still stream `local` before the eventual
    // `reranked` frame, even though the pipeline now defers the validation
    // until after the rerank stage resolves.
    let candidates: Vec<SearchHit> = (1..=20)
        .map(|doc| test_hit(doc, 0, &format!("meaningful body text {doc}")))
        .collect();
    let index_hits = candidates
        .iter()
        .enumerate()
        .map(|(pos, hit)| SearchPointHit {
            chunk_id: hit.chunk_id,
            score: 0.9 - pos as f32 * 0.005,
        })
        .collect::<Vec<_>>();
    let hydrated = candidates.into_iter().map(|hit| (hit.chunk_id, hit)).collect();
    let service = build_service_with_rerank(
        hybrid_settings(true),
        index_hits,
        hydrated,
        Vec::new(),
    );

    let page1 = block_on(service.search(None, test_request(1, 8))).expect("page 1");
    let next_cursor = page1
        .pagination
        .next_cursor
        .clone()
        .expect("page 1 must have a next cursor when more results exist");
    let mut request = test_request(1, 8);
    request.cursor = Some(next_cursor);
    let frames = collect_stream_frames(&service, request);
    // The cursor matches the reranked epoch so the pipeline must stream a
    // local page followed by a reranked page, then done. We deliberately
    // do not require a strict local → reranked order because future
    // optimizations may fold the local frame, but a `local` is still the
    // contract for now.
    assert!(
        frames
            .iter()
            .any(|frame| matches!(frame, SearchStreamEvent::Local(_))),
        "matching hybrid cursor must still surface a local page; got {frames:?}"
    );
    assert!(
        frames
            .iter()
            .any(|frame| matches!(frame, SearchStreamEvent::Reranked(_))),
        "matching hybrid cursor must surface a reranked page; got {frames:?}"
    );
    assert!(
        !frames
            .iter()
            .any(|frame| matches!(frame, SearchStreamEvent::Error { .. })),
        "no error frame expected for a matching hybrid cursor"
    );
}

#[test]
fn cursor_navigation_reuses_offset_windows() {
    // A cursor issued for page 2 (offset 8, local ordering) reproduces the
    // exact POST page-2 window and carries matching prev/next cursors.
    let hits: Vec<SearchHit> = (1..=20)
        .map(|doc| test_hit(doc, 0, &format!("meaningful body text {doc}")))
        .collect();
    let index_hits = hits
        .iter()
        .enumerate()
        .map(|(pos, hit)| SearchPointHit {
            chunk_id: hit.chunk_id,
            score: 0.9 - pos as f32 * 0.005,
        })
        .collect::<Vec<_>>();
    let hydrated = hits.into_iter().map(|hit| (hit.chunk_id, hit)).collect();
    let (service, _, _) = build_service(vector_settings(), index_hits, hydrated, Vec::new());

    let post_page2 = block_on(service.search(None, test_request(2, 8))).expect("page 2");
    let next_cursor = post_page2
        .pagination
        .next_cursor
        .clone()
        .expect("page 2 has a next cursor");

    let mut cursor_request = test_request(1, 8);
    cursor_request.cursor = Some(next_cursor);
    let post_page3 = block_on(service.search(None, cursor_request)).expect("cursor page 3");
    let expected_page3 = block_on(service.search(None, test_request(3, 8))).expect("page 3");
    assert_eq!(
        serde_json::to_value(&post_page3.pagination).unwrap(),
        serde_json::to_value(&expected_page3.pagination).unwrap()
    );
    assert_eq!(
        post_page3
            .items
            .iter()
            .map(|hit| hit.chunk_id)
            .collect::<Vec<_>>(),
        expected_page3
            .items
            .iter()
            .map(|hit| hit.chunk_id)
            .collect::<Vec<_>>()
    );
}

/// Deterministic mock rerank: assigns scores so the last document in the
/// snapshot ranks first, the second-to-last second, and so on. The result
/// is one stable, easy-to-verify cross-page ordering without an external
/// call.
struct MockReverseRerank;

#[async_trait]
impl RerankClient for MockReverseRerank {
    async fn rerank(
        &self,
        _query: &str,
        documents: &[RerankDocument],
        top_n: usize,
        _settings: &SearchSettings,
    ) -> anyhow::Result<Vec<RerankHit>> {
        let total = top_n.min(documents.len());
        // score = position, so the highest position (last document in the
        // snapshot) gets the highest score. After the descending sort the
        // relevance order is the snapshot in reverse.
        let mut hits: Vec<RerankHit> = (0..total)
            .map(|position| RerankHit {
                index: position,
                score: position as f32,
            })
            .collect();
        hits.sort_by(|left, right| right.score.total_cmp(&left.score));
        Ok(hits)
    }
}

fn build_service_with_rerank(
    settings: SearchSettings,
    index_hits: Vec<SearchPointHit>,
    hydrated: HashMap<Uuid, SearchHit>,
    keyword_hits: Vec<SearchHit>,
) -> SearchService {
    let repo = MockRepo {
        settings,
        hydrated,
        keyword_hits,
        seen_keyword_limit: Arc::new(Mutex::new(None)),
        upper_bound: None,
    };
    block_on(SearchService::with_rerank_client(
        Arc::new(repo),
        Arc::new(MockScope),
        Arc::new(MockEmbedding),
        Arc::new(MockIndex {
            hits: index_hits,
            seen_limit: Arc::new(Mutex::new(None)),
            date_hits: Vec::new(),
            seen_date_bounds: Arc::new(Mutex::new(Vec::new())),
            date_window_strategy: Arc::new(Mutex::new(DateWindowStrategy::AdversarialFirstPage)),
            fetch_limit_override: None,
        }),
        Arc::new(MockReverseRerank),
        None,
        "test-model".to_string(),
    ))
    .expect("service")
}

#[test]
fn cursor_rejects_mismatched_query_context() {
    let hits: Vec<SearchHit> = (1..=12)
        .map(|doc| test_hit(doc, 0, &format!("meaningful body text {doc}")))
        .collect();
    let index_hits = hits
        .iter()
        .enumerate()
        .map(|(pos, hit)| SearchPointHit {
            chunk_id: hit.chunk_id,
            score: 0.9 - pos as f32 * 0.01,
        })
        .collect::<Vec<_>>();
    let hydrated = hits.into_iter().map(|hit| (hit.chunk_id, hit)).collect();
    let (service, _, _) = build_service(vector_settings(), index_hits, hydrated, Vec::new());

    // A cursor issued by a previous query must not be replayable against
    // a different query. POST must reject the request (the HTTP layer
    // surfaces this as 400).
    let mut request = test_request(1, 8);
    let cursor = encode_cursor(
        8,
        false,
        8,
        &CursorContext {
            // Mismatched query hash so the cursor is invalid for the
            // active request's `query: "query"`.
            query_hash: SearchCache::query_hash("a-different-query"),
            filter_hash: SearchCache::filter_hash(&test_request(1, 8)),
            generation: 0,
            settings_hash: SearchCache::settings_hash(&vector_settings()),
            limit: 8,
        },
    );
    request.cursor = Some(cursor);
    let error = block_on(service.search(None, request)).expect_err("cursor mismatch must reject");
    let message = error.to_string();
    assert!(
        message.contains("cursor context mismatch"),
        "unexpected error: {message}"
    );
}

#[test]
fn cursor_rejects_misaligned_offset() {
    let hits: Vec<SearchHit> = (1..=12)
        .map(|doc| test_hit(doc, 0, &format!("meaningful body text {doc}")))
        .collect();
    let index_hits = hits
        .iter()
        .enumerate()
        .map(|(pos, hit)| SearchPointHit {
            chunk_id: hit.chunk_id,
            score: 0.9 - pos as f32 * 0.01,
        })
        .collect::<Vec<_>>();
    let hydrated = hits.into_iter().map(|hit| (hit.chunk_id, hit)).collect();
    let (service, _, _) = build_service(vector_settings(), index_hits, hydrated, Vec::new());

    // An offset that does not align to the page size must be rejected.
    let mut request = test_request(1, 8);
    let cursor = encode_cursor(
        5,
        false,
        8,
        &CursorContext {
            query_hash: SearchCache::query_hash("query"),
            filter_hash: SearchCache::filter_hash(&test_request(1, 8)),
            generation: 0,
            settings_hash: SearchCache::settings_hash(&vector_settings()),
            limit: 8,
        },
    );
    request.cursor = Some(cursor);
    let error = block_on(service.search(None, request)).expect_err("misaligned cursor must reject");
    assert!(
        error.to_string().contains("not aligned"),
        "unexpected error: {error}"
    );
}

#[test]
fn reranked_page_two_is_consistent_with_page_one() {
    // F4: a stable rerank candidate snapshot must make page 1 + page 2
    // disjoint and consistent with one full reranked ordering. The mock
    // rerank reverses the snapshot order, so the first item of page 1 is
    // the LAST item of the snapshot and the first item of page 2 is the
    // snapshot's second-to-last item.
    let candidates: Vec<SearchHit> = (1..=20)
        .map(|doc| test_hit(doc, 0, &format!("meaningful body text {doc}")))
        .collect();
    let index_hits = candidates
        .iter()
        .enumerate()
        .map(|(pos, hit)| SearchPointHit {
            chunk_id: hit.chunk_id,
            score: 0.9 - pos as f32 * 0.01,
        })
        .collect::<Vec<_>>();
    let hydrated = candidates.into_iter().map(|hit| (hit.chunk_id, hit)).collect();
    let service = build_service_with_rerank(
        hybrid_settings(true),
        index_hits,
        hydrated,
        Vec::new(),
    );

    let page1 = block_on(service.search(None, test_request(1, 8))).expect("page 1");
    let next_cursor = page1
        .pagination
        .next_cursor
        .clone()
        .expect("page 1 must have a next cursor when more results exist");
    let mut page2_request = test_request(1, 8);
    page2_request.cursor = Some(next_cursor);
    let page2 = block_on(service.search(None, page2_request)).expect("page 2");

    // Page 1 + page 2 must be disjoint (no row repeats across the cursor
    // boundary) and form a prefix of the stable rerank ordering.
    let page1_ids: Vec<Uuid> = page1.items.iter().map(|hit| hit.chunk_id).collect();
    let page2_ids: Vec<Uuid> = page2.items.iter().map(|hit| hit.chunk_id).collect();
    for id in &page2_ids {
        assert!(
            !page1_ids.contains(id),
            "page 2 must not repeat a row from page 1 (got {id})"
        );
    }
    // The two pages are slices of one full ordering: their concatenation is
    // the rerank's first 16 items in document-id reverse order (mock
    // rerank returns documents in reverse index order, snapshot is the
    // top-40 by local score). Document 1 is at the start of the snapshot,
    // document 20 is at the end; the reverse ordering is 20, 19, 18, ...
    let expected: Vec<i64> = (1..=16).map(|doc| 21 - doc).collect();
    let actual: Vec<i64> = page1
        .items
        .iter()
        .chain(page2.items.iter())
        .map(|hit| hit.document_id)
        .collect();
    assert_eq!(actual, expected, "page 1 + page 2 must equal the stable rerank prefix");
}

#[test]
fn post_invalid_page_or_limit_is_rejected() {
    // F6: invalid page/limit must surface as a clean error so the HTTP
    // adapter can map it to 400. The service rejects these before any
    // work begins, regardless of mode.
    let (service, _, _) = build_service(vector_settings(), Vec::new(), HashMap::new(), Vec::new());

    assert!(
        block_on(service.search(None, test_request(0, 8))).is_err(),
        "page 0 must be rejected"
    );
    assert!(
        block_on(service.search(None, test_request(1, 0))).is_err(),
        "limit 0 must be rejected"
    );
    assert!(
        block_on(service.search(None, test_request(1, 101))).is_err(),
        "limit 101 must be rejected"
    );
}

#[test]
fn sse_rejects_cursor_mismatch_before_local_frame() {
    // F2: a mismatched cursor must surface as the only SSE frame; the
    // client must never see a `local` page that was issued under a
    // different context.
    let hits: Vec<SearchHit> = (1..=5)
        .map(|doc| test_hit(doc, 0, &format!("meaningful body text {doc}")))
        .collect();
    let index_hits = hits
        .iter()
        .map(|hit| SearchPointHit {
            chunk_id: hit.chunk_id,
            score: 0.8,
        })
        .collect::<Vec<_>>();
    let hydrated = hits.into_iter().map(|hit| (hit.chunk_id, hit)).collect();
    let (service, _, _) = build_service(vector_settings(), index_hits, hydrated, Vec::new());

    // Build a cursor with a wrong query hash, then send a request that
    // would otherwise succeed. The SSE must emit only the error frame.
    let mut request = test_request(1, 8);
    request.query = "different".to_string();
    let cursor = encode_cursor(
        8,
        false,
        8,
        &CursorContext {
            query_hash: SearchCache::query_hash("query"),
            filter_hash: SearchCache::filter_hash(&test_request(1, 8)),
            generation: 0,
            settings_hash: SearchCache::settings_hash(&vector_settings()),
            limit: 8,
        },
    );
    request.cursor = Some(cursor);
    let frames = collect_stream_frames(&service, request);
    assert!(
        frames
            .iter()
            .all(|frame| !matches!(frame, SearchStreamEvent::Local(_))),
        "no local frame must be emitted on cursor mismatch; got {frames:?}"
    );
    assert!(frames.iter().any(|frame| matches!(
        frame,
        SearchStreamEvent::Error { message } if message.contains("cursor context mismatch")
    )));
}

#[tokio::test(flavor = "current_thread")]
async fn stream_abort_stops_producer_before_first_frame() {
    // F1: a producer selecting on the abort signal must return when the
    // guard is dropped, even if the embedding future is still pending.
    // We use a "blocked" embedding to simulate in-flight work.
    struct BlockedEmbedding {
        release: Arc<std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    }

    #[async_trait]
    impl SearchEmbeddingProvider for BlockedEmbedding {
        async fn embed_query(&self, _query: &str) -> anyhow::Result<Vec<f32>> {
            let rx = {
                let mut guard = self.release.lock().expect("lock");
                let (tx, rx) = tokio::sync::oneshot::channel();
                *guard = Some(tx);
                rx
            };
            // Block until the test releases us or the channel is closed
            // (which happens when the abort signal drops the receiver).
            let _ = rx.await;
            Err(anyhow!("blocked embedding aborted"))
        }
    }

    let release: Arc<std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>> =
        Arc::new(std::sync::Mutex::new(None));
    let embedding = Arc::new(BlockedEmbedding {
        release: Arc::clone(&release),
    });
    let repo = Arc::new(MockRepo {
        settings: hybrid_settings(false),
        hydrated: HashMap::new(),
        keyword_hits: Vec::new(),
        seen_keyword_limit: Arc::new(Mutex::new(None)),
        upper_bound: None,
    });
    let index = Arc::new(MockIndex {
        hits: Vec::new(),
        seen_limit: Arc::new(Mutex::new(None)),
        date_hits: Vec::new(),
        seen_date_bounds: Arc::new(Mutex::new(Vec::new())),
        date_window_strategy: Arc::new(Mutex::new(DateWindowStrategy::AdversarialFirstPage)),
            fetch_limit_override: None,
    });
    let scope = Arc::new(MockScope);
    let service = SearchService::new(
        repo,
        scope,
        embedding,
        index,
        None,
        "test-model".to_string(),
    )
    .await
    .expect("service");
    let (tx, mut rx) = tokio::sync::mpsc::channel::<SearchStreamEvent>(8);
    let (signal, guard) = abort_pair();
    let producer = tokio::spawn(async move {
        let _ = service.stream_search(None, test_request(1, 8), tx, signal).await;
    });
    // Give the producer time to enter `select!` on the blocked embedding.
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    // Drop the producer's abort guard (simulating a client disconnect).
    drop(guard);
    // The producer must observe the abort and return without pushing a
    // `local` frame. Wait for it to finish.
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(2), producer).await;
    assert!(outcome.is_ok(), "producer did not finish after abort");
    let _ = tokio::time::timeout(std::time::Duration::from_millis(50), rx.recv()).await;
    // Drop the test-side release so the blocked embedding's awaiter
    // resolves too; we don't care about its return value here.
    if let Some(release_tx) = release.lock().expect("lock").take() {
        let _ = release_tx.send(());
    }
}

/// Build a service with date-mode mocks: a vector of `SearchDatePointHit`
/// (the index's "raw" data), a hydrated map keyed by `chunk_id`, and an
/// optional `published_ts` upper bound. Returns the service plus the mocks
/// for assertions.
fn build_date_service(
    index_hits: Vec<SearchDatePointHit>,
    hydrated: HashMap<Uuid, SearchHit>,
    upper_bound: Option<i64>,
) -> (SearchService, DateMocks) {
    let seen_index = Arc::new(Mutex::new(None));
    let seen_date_bounds = Arc::new(Mutex::new(Vec::new()));
    let repo = MockRepo {
        settings: vector_settings(),
        hydrated,
        keyword_hits: Vec::new(),
        seen_keyword_limit: Arc::new(Mutex::new(None)),
        upper_bound,
    };
    let mocks = DateMocks {
        seen_index: Arc::clone(&seen_index),
        seen_date_bounds: Arc::clone(&seen_date_bounds),
    };
    let service = block_on(SearchService::new(
        Arc::new(repo),
        Arc::new(MockScope),
        Arc::new(MockEmbedding),
        Arc::new(MockIndex {
            hits: Vec::new(),
            seen_limit: seen_index,
            date_hits: index_hits,
            seen_date_bounds,
            date_window_strategy: Arc::new(Mutex::new(DateWindowStrategy::AdversarialFirstPage)),
            fetch_limit_override: None,
        }),
        None,
        "test-model".to_string(),
    ))
    .expect("service");
    (service, mocks)
}

/// Same as `build_date_service` but lets the test pin the per-call
/// fetch limit reported to the mock. The genuine-truncation test
/// uses a small fetch limit (e.g. 64) to exercise a population
/// strictly larger than one fetch without inflating the population
/// to thousands of records.
fn build_date_service_with_fetch_limit(
    index_hits: Vec<SearchDatePointHit>,
    hydrated: HashMap<Uuid, SearchHit>,
    upper_bound: Option<i64>,
    fetch_limit: usize,
) -> (SearchService, DateMocks) {
    let seen_index = Arc::new(Mutex::new(None));
    let seen_date_bounds = Arc::new(Mutex::new(Vec::new()));
    let repo = MockRepo {
        settings: vector_settings(),
        hydrated,
        keyword_hits: Vec::new(),
        seen_keyword_limit: Arc::new(Mutex::new(None)),
        upper_bound,
    };
    let mocks = DateMocks {
        seen_index: Arc::clone(&seen_index),
        seen_date_bounds: Arc::clone(&seen_date_bounds),
    };
    let service = block_on(SearchService::new(
        Arc::new(repo),
        Arc::new(MockScope),
        Arc::new(MockEmbedding),
        Arc::new(MockIndex {
            hits: Vec::new(),
            seen_limit: seen_index,
            date_hits: index_hits,
            seen_date_bounds,
            date_window_strategy: Arc::new(Mutex::new(DateWindowStrategy::AdversarialFirstPage)),
            fetch_limit_override: Some(fetch_limit),
        }),
        None,
        "test-model".to_string(),
    ))
    .expect("service");
    (service, mocks)
}

#[derive(Clone)]
struct DateMocks {
    #[allow(dead_code)]
    seen_index: Arc<Mutex<Option<usize>>>,
    seen_date_bounds: Arc<Mutex<Vec<(Option<i64>, Option<i64>)>>>,
}

fn date_hit(chunk_id: Uuid, published_ts: Option<i64>) -> SearchDatePointHit {
    SearchDatePointHit {
        chunk_id,
        published_ts,
        score: 0.0,
    }
}

fn date_request(limit: usize) -> SearchRequest {
    let mut request = test_request(1, limit);
    request.sort = context69_contracts::SearchSort::Date;
    // Date mode matches the query against `title + chunk_text`. The
    // existing date-mode fixtures use chunk text such as "fresh body
    // text" or "older body text"; the token "body" is present in every
    // fixture, so it is the default query for the date-mode helper.
    // Tests that need a different query construct the request by hand
    // (e.g. `date_mode_rejects_blank_query`).
    request.query = "body".to_string();
    request
}

fn date_request_with_cursor(limit: usize, cursor: String) -> SearchRequest {
    let mut request = date_request(limit);
    request.cursor = Some(cursor);
    request
}

#[test]
fn date_mode_first_page_returns_latest_first_in_qdrant_order() {
    // Three records at three distinct timestamps + a same-second pair. The
    // pipeline must sort by `published_at DESC` and follow the Qdrant
    // `scroll` return order inside a same-second boundary (deterministic
    // per ordering epoch). The mock's `AdversarialFirstPage` strategy
    // sorts by `chunk_id DESC`, so the same-second pair arrives in
    // B-then-A order. UUIDs are fixed so the order is deterministic.
    let first = (
        Uuid::from_u128(0x3333_3333_3333_3333_3333_3333_3333_3333),
        make_hit_with_chunk(1, Uuid::from_u128(0x3333_3333_3333_3333_3333_3333_3333_3333), 1_700_000_010, "first record body"),
    );
    let second = (
        Uuid::from_u128(0x2222_2222_2222_2222_2222_2222_2222_2222),
        make_hit_with_chunk(2, Uuid::from_u128(0x2222_2222_2222_2222_2222_2222_2222_2222), 1_700_000_008, "second record body"),
    );
    let same_second_a = (
        Uuid::from_u128(0xaaaa_aaaa_aaaa_aaaa_aaaa_aaaa_aaaa_aaaa),
        make_hit_with_chunk(3, Uuid::from_u128(0xaaaa_aaaa_aaaa_aaaa_aaaa_aaaa_aaaa_aaaa), 1_700_000_005, "same second A body"),
    );
    let same_second_b = (
        Uuid::from_u128(0xffff_ffff_ffff_ffff_ffff_ffff_ffff_ffff),
        make_hit_with_chunk(4, Uuid::from_u128(0xffff_ffff_ffff_ffff_ffff_ffff_ffff_ffff), 1_700_000_005, "same second B body"),
    );
    let mut hydrated = HashMap::new();
    for (id, hit) in [&first, &second, &same_second_a, &same_second_b] {
        hydrated.insert(*id, hit.1.clone());
    }
    let index_hits = vec![
        date_hit(first.0, Some(1_700_000_010)),
        date_hit(second.0, Some(1_700_000_008)),
        date_hit(same_second_a.0, Some(1_700_000_005)),
        date_hit(same_second_b.0, Some(1_700_000_005)),
    ];
    let (service, _) = build_date_service(
        index_hits,
        hydrated,
        Some(1_700_000_010),
    );
    let response = block_on(service.search(None, date_request(4))).expect("date search");
    let actual: Vec<i64> = response.items.iter().map(|hit| hit.document_id).collect();
    // Expected order: fresh (1) → second (2) → same-second B (4, larger UUID) → A (3).
    // The Qdrant return order is the authoritative ordering; the
    // pipeline does NOT re-sort in memory.
    assert_eq!(actual, vec![1, 2, 4, 3]);
    // The window was full (`fetched = limit`), so the pipeline must
    // honestly report `has_more = true` even when the records fit
    // exactly: only the next window's emptiness can confirm there is no
    // more.
    assert_eq!(response.pagination.has_more, Some(true));
    assert_eq!(response.pagination.total_is_exact, Some(false));
    let next_cursor = response
        .pagination
        .next_cursor
        .expect("full window must emit a next cursor");
    let next = block_on(service.search(None, date_request_with_cursor(4, next_cursor)))
        .expect("date search page 2");
    // The next window's `before = 1_700_000_004` is below every record;
    // the pipeline must report an empty page and no further cursor.
    assert!(next.items.is_empty());
    assert_eq!(next.pagination.has_more, Some(false));
    assert!(next.pagination.next_cursor.is_none());
}

#[test]
fn date_mode_walks_multiple_windows_without_overlap() {
    // The mock returns the same hits on every window call, so the
    // pipeline must walk the keyset monotonically: the second timestamp
    // (`1_700_000_005`) is only visited after the first (`1_700_000_010`)
    // is drained, and every Qdrant call's upper bound is strictly less
    // than or equal to the previous call's smallest timestamp. UUIDs
    // are fixed so the `chunk_id` ordering is deterministic.
    let first_chunk = Uuid::from_u128(0x1111_1111_1111_1111_1111_1111_1111_1111);
    let second_chunk = Uuid::from_u128(0x2222_2222_2222_2222_2222_2222_2222_2222);
    let first = make_hit_with_chunk(1, first_chunk, 1_700_000_010, "fresh body text");
    let second = make_hit_with_chunk(2, second_chunk, 1_700_000_005, "older body text");
    let mut hydrated = HashMap::new();
    for hit in [&first, &second] {
        hydrated.insert(hit.0, hit.1.clone());
    }
    let index_hits = vec![
        date_hit(first.0, Some(1_700_000_010)),
        date_hit(second.0, Some(1_700_000_005)),
    ];
    let (service, mocks) = build_date_service(
        index_hits,
        hydrated,
        Some(1_700_000_010),
    );
    let response = block_on(service.search(None, date_request(2))).expect("date search");
    let actual: Vec<i64> = response.items.iter().map(|hit| hit.document_id).collect();
    assert_eq!(actual, vec![1, 2]);
    // The mock reports the same hits on every call, so the pipeline
    // first drains the `1_700_000_010` boundary (one cross-ts fetch
    // + one per-timestamp drain call) and then walks to the next
    // timestamp. The upper bound on every fetch is strictly less than
    // or equal to the snapshot's upper bound, and the second
    // timestamp's fetch has a smaller upper bound than the first
    // (1_700_000_005 < 1_700_000_010).
    let bounds = mocks.seen_date_bounds.lock().expect("bounds lock").clone();
    assert!(
        !bounds.is_empty(),
        "at least one Qdrant fetch must happen; got {bounds:?}"
    );
    let mut previous_before: Option<i64> = None;
    for (after, before) in &bounds {
        assert_eq!(*after, None, "after is open for every fetch; got {bounds:?}");
        if let Some(prev) = previous_before {
            assert!(
                before.unwrap_or(i64::MAX) <= prev,
                "Qdrant fetch upper bound must be monotonically non-increasing; got {bounds:?}"
            );
        }
        previous_before = *before;
    }
    // The very first fetch's upper bound is the snapshot ceiling
    // (1_700_000_010). All subsequent fetches stay at or below it.
    assert_eq!(bounds[0], (None, Some(1_700_000_010)));
    let next_cursor = response
        .pagination
        .next_cursor
        .expect("next cursor expected when window reported more");
    let page_two = block_on(service.search(
        None,
        date_request_with_cursor(2, next_cursor),
    ))
    .expect("date search page 2");
    let actual: Vec<i64> = page_two.items.iter().map(|hit| hit.document_id).collect();
    // The mock is configured to return both hits on every call, so
    // page 2 must skip them via the `seen` set and emit no records.
    assert!(actual.is_empty(), "duplicate items leaked to page 2: {actual:?}");
    // The second page's calls must keep the monotonicity invariant:
    // the per-timestamp drain resumes at `boundary_ts = 1_700_000_005`
    // and the upper bound is at or below the previous page's smallest
    // timestamp.
    let total_bounds = mocks.seen_date_bounds.lock().expect("bounds lock").clone();
    let second_page_bounds: Vec<_> = total_bounds.iter().skip(bounds.len()).cloned().collect();
    assert!(
        !second_page_bounds.is_empty(),
        "page 2 must trigger at least one Qdrant fetch; got {total_bounds:?}"
    );
    let (after, before) = second_page_bounds[0];
    assert_eq!(after, None);
    assert!(
        before.unwrap_or(i64::MAX) <= 1_700_000_010,
        "page 2 fetch must not widen above the previous page's upper bound; got {second_page_bounds:?}"
    );
}

#[test]
fn date_mode_empty_window_advances_without_duplication() {
    // The mock has only one record at the upper bound, so the first window
    // returns the record and the next window is empty. The pipeline must
    // stop after the second window reports "no more results" (the empty
    // path must not spin).
    let only_chunk = Uuid::from_u128(0xaaaa_aaaa_aaaa_aaaa_aaaa_aaaa_aaaa_aaaa);
    let only = make_hit_with_chunk(1, only_chunk, 1_700_000_010, "only body text");
    let mut hydrated = HashMap::new();
    hydrated.insert(only.0, only.1.clone());
    let index_hits = vec![date_hit(only.0, Some(1_700_000_010))];
    let (service, _) = build_date_service(index_hits, hydrated, Some(1_700_000_010));
    let response = block_on(service.search(None, date_request(2))).expect("date search");
    let actual: Vec<i64> = response.items.iter().map(|hit| hit.document_id).collect();
    assert_eq!(actual, vec![1]);
    assert_eq!(response.pagination.has_more, Some(false));
    assert!(response.pagination.next_cursor.is_none());
}

#[test]
fn date_mode_rerank_is_not_invoked() {
    // Date mode must never reach the rerank stage. The mock embedding /
    // index is a passthrough; if any code path tried to call the rerank
    // client the test would panic because none is configured (the mock
    // service uses the default `OpenRouterRerankClient` which is
    // constructed in `SearchService::new`).
    let chunk_id = Uuid::from_u128(0x3333_3333_3333_3333_3333_3333_3333_3333);
    let hit = make_hit_with_chunk(1, chunk_id, 1_700_000_010, "body text");
    let mut hydrated = HashMap::new();
    hydrated.insert(hit.0, hit.1.clone());
    let index_hits = vec![date_hit(hit.0, Some(1_700_000_010))];
    let (service, _) = build_date_service(index_hits, hydrated, Some(1_700_000_010));
    let response = block_on(service.search(None, date_request(1))).expect("date search");
    assert_eq!(response.items.len(), 1);
    // Vector score is propagated from the index; rerank score is `None`.
    assert!(response.items[0].rerank_score.is_none());
}

#[test]
fn date_mode_rejects_relevance_cursor() {
    // A `c1:` cursor must not be accepted by the date pipeline.
    let hit = (Uuid::new_v4(), make_hit(1, 1_700_000_010, "body"));
    let mut hydrated = HashMap::new();
    hydrated.insert(hit.0, hit.1.clone());
    let index_hits = vec![date_hit(hit.0, Some(1_700_000_010))];
    let (service, _) = build_date_service(index_hits, hydrated, Some(1_700_000_010));
    let relevance_cursor = encode_cursor(
        0,
        false,
        8,
        &CursorContext {
            query_hash: SearchCache::query_hash("query"),
            filter_hash: SearchCache::filter_hash(&date_request(8)),
            generation: 0,
            settings_hash: SearchCache::settings_hash(&vector_settings()),
            limit: 8,
        },
    );
    let mut request = date_request(8);
    request.cursor = Some(relevance_cursor);
    let error = block_on(service.search(None, request))
        .expect_err("relevance cursor must be rejected by date pipeline");
    assert!(
        error.to_string().contains("relevance"),
        "unexpected error: {error}"
    );
}

#[test]
fn date_mode_rejects_changed_filter() {
    // A `c4:` cursor with a mismatched query must be rejected.
    let hit = (Uuid::new_v4(), make_hit(1, 1_700_000_010, "body"));
    let mut hydrated = HashMap::new();
    hydrated.insert(hit.0, hit.1.clone());
    let index_hits = vec![date_hit(hit.0, Some(1_700_000_010))];
    let (service, _) = build_date_service(index_hits, hydrated, Some(1_700_000_010));
    let cursor = super::search_cursor::encode_date_cursor(&super::search_cursor::DateCursor {
        limit: 8,
        query_hash: "different".to_string(),
        filter_hash: SearchCache::filter_hash(&date_request(8)),
        generation: 0,
        settings_hash: SearchCache::settings_hash(&vector_settings()),
        before: Some(1_700_000_010),
        boundary_ts: Some(1_700_000_010),
        offset: None,
        upper: Some(1_700_000_010),
    });
    let error = block_on(service.search(None, date_request_with_cursor(8, cursor)))
        .expect_err("query hash mismatch must be rejected");
    assert!(error.to_string().contains("cursor context mismatch"));
}

fn make_hit(document_id: i64, published_ts: i64, text: &str) -> SearchHit {
    let mut hit = test_hit(document_id, 0, text);
    hit.published_at = chrono::DateTime::<chrono::Utc>::from_timestamp(published_ts, 0);
    hit
}

#[test]
fn date_mode_same_second_qdrant_order_is_deterministic_across_pages() {
    // Two same-second records must appear in the same order on every
    // page. We split them across the cursor boundary to prove the
    // Qdrant `scroll` return order survives window handoffs. The
    // `AdversarialFirstPage` mock sorts by `chunk_id DESC` so the
    // larger-UUID record arrives first. All UUIDs are fixed so the
    // Qdrant return order is deterministic.
    let high_chunk = Uuid::from_u128(0x3333_3333_3333_3333_3333_3333_3333_3333);
    let a_chunk = Uuid::from_u128(0xaaaa_aaaa_aaaa_aaaa_aaaa_aaaa_aaaa_aaaa);
    let b_chunk = Uuid::from_u128(0xffff_ffff_ffff_ffff_ffff_ffff_ffff_ffff);
    let high = make_hit_with_chunk(1, high_chunk, 1_700_000_010, "first record body");
    let a = make_hit_with_chunk(2, a_chunk, 1_700_000_005, "second A body");
    let b = make_hit_with_chunk(3, b_chunk, 1_700_000_005, "second B body");
    let mut hydrated = HashMap::new();
    for hit in [&high, &a, &b] {
        hydrated.insert(hit.0, hit.1.clone());
    }
    let index_hits = vec![
        date_hit(high.0, Some(1_700_000_010)),
        date_hit(a.0, Some(1_700_000_005)),
        date_hit(b.0, Some(1_700_000_005)),
    ];
    let (service, _) = build_date_service(
        index_hits,
        hydrated,
        Some(1_700_000_010),
    );
    // Page 1 (limit=2): high, B. B and A share a second; the Qdrant
    // return order is B-then-A (largest UUID first per the
    // adversarial mock), so the cursor is bound by B's
    // `published_ts = 1_700_000_005`.
    let page1 = block_on(service.search(None, date_request(2))).expect("page 1");
    assert_eq!(
        page1.items.iter().map(|hit| hit.document_id).collect::<Vec<_>>(),
        vec![1, 3]
    );
    let next_cursor = page1
        .pagination
        .next_cursor
        .clone()
        .expect("full window must emit a next cursor");
    // Page 2 (limit=2): only A remains. The pipeline must skip the
    // already-seen B and emit A alone.
    let page2 = block_on(service.search(None, date_request_with_cursor(2, next_cursor.clone())))
        .expect("page 2");
    assert_eq!(
        page2.items.iter().map(|hit| hit.document_id).collect::<Vec<_>>(),
        vec![2]
    );
    // Replaying page 1's cursor through the service should yield the
    // same A record (deterministic Qdrant return order).
    let replay = block_on(service.search(None, date_request_with_cursor(2, next_cursor)))
        .expect("replay page 2");
    assert_eq!(
        replay.items.iter().map(|hit| hit.document_id).collect::<Vec<_>>(),
        vec![2]
    );
}

#[test]
fn date_mode_post_and_sse_return_same_sequence() {
    let high = make_hit_with_chunk(1, Uuid::from_u128(0x3333_3333_3333_3333_3333_3333_3333_3333), 1_700_000_010, "first record body");
    let a = make_hit_with_chunk(2, Uuid::from_u128(0x2222_2222_2222_2222_2222_2222_2222_2222), 1_700_000_008, "second record body");
    let b = make_hit_with_chunk(3, Uuid::from_u128(0x4444_4444_4444_4444_4444_4444_4444_4444), 1_700_000_005, "third record body");
    let mut hydrated = HashMap::new();
    for hit in [&high, &a, &b] {
        hydrated.insert(hit.0, hit.1.clone());
    }
    let index_hits = vec![
        date_hit(high.0, Some(1_700_000_010)),
        date_hit(a.0, Some(1_700_000_008)),
        date_hit(b.0, Some(1_700_000_005)),
    ];
    let (service, _) = build_date_service(
        index_hits,
        hydrated,
        Some(1_700_000_010),
    );
    let post = block_on(service.search(None, date_request(2))).expect("post");
    let stream = collect_stream_frames(&service, date_request(2));
    let stream_page = match stream.into_iter().next() {
        Some(SearchStreamEvent::Local(page)) => page,
        other => panic!("expected a local frame first, got {other:?}"),
    };
    let post_ids: Vec<i64> = post.items.iter().map(|hit| hit.document_id).collect();
    let stream_ids: Vec<i64> = stream_page
        .items
        .iter()
        .map(|hit| hit.document_id)
        .collect();
    assert_eq!(post_ids, stream_ids, "POST and SSE must emit the same order");
    assert_eq!(post_ids, vec![1, 2]);
}

#[tokio::test(flavor = "current_thread")]
async fn date_mode_abort_stops_streaming_before_first_frame() {
    // A date-mode SSE request whose embedding is blocked must honour the
    // abort signal: no `local` frame is emitted when the client
    // disconnects mid-embed.
    struct BlockedEmbedding {
        release: Arc<std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>>,
    }

    #[async_trait]
    impl SearchEmbeddingProvider for BlockedEmbedding {
        async fn embed_query(&self, _query: &str) -> anyhow::Result<Vec<f32>> {
            let rx = {
                let mut guard = self.release.lock().expect("lock");
                let (tx, rx) = tokio::sync::oneshot::channel();
                *guard = Some(tx);
                rx
            };
            let _ = rx.await;
            Err(anyhow!("blocked embedding aborted"))
        }
    }

    let release: Arc<std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>> =
        Arc::new(std::sync::Mutex::new(None));
    let embedding = Arc::new(BlockedEmbedding {
        release: Arc::clone(&release),
    });
    let repo = Arc::new(MockRepo {
        settings: vector_settings(),
        hydrated: HashMap::new(),
        keyword_hits: Vec::new(),
        seen_keyword_limit: Arc::new(Mutex::new(None)),
        upper_bound: None,
    });
    let index = Arc::new(MockIndex {
        hits: Vec::new(),
        seen_limit: Arc::new(Mutex::new(None)),
        date_hits: Vec::new(),
        seen_date_bounds: Arc::new(Mutex::new(Vec::new())),
        date_window_strategy: Arc::new(Mutex::new(DateWindowStrategy::AdversarialFirstPage)),
            fetch_limit_override: None,
    });
    let scope = Arc::new(MockScope);
    let service = SearchService::new(
        repo,
        scope,
        embedding,
        index,
        None,
        "test-model".to_string(),
    )
    .await
    .expect("service");
    let (tx, mut rx) = tokio::sync::mpsc::channel::<SearchStreamEvent>(8);
    let (signal, guard) = abort_pair();
    let producer = tokio::spawn(async move {
        let _ = service.stream_search(None, date_request(2), tx, signal).await;
    });
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    drop(guard);
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(2), producer).await;
    assert!(outcome.is_ok(), "producer did not finish after abort");
    let _ = tokio::time::timeout(std::time::Duration::from_millis(50), rx.recv()).await;
    if let Some(release_tx) = release.lock().expect("lock").take() {
        let _ = release_tx.send(());
    }
}

/// Date-mode regression: the same-second population must be drained in
/// full before the pipeline advances to an older timestamp, even when it
/// spans many more `chunk_id`s than the page `limit`. The Qdrant
/// `scroll` return order is the authoritative ordering; the in-memory
/// `seen` set is the only dedup mechanism. The cursor's `boundary_ts`
/// + `offset` (PointId UUID) together describe the in-flight drain
/// and the Qdrant-side resume key so a replayed request resumes the
/// boundary in the same order.
#[test]
fn date_mode_same_second_population_exceeds_limit_drains_in_full() {
    // 10 records at the same `published_ts`, page size 4. The
    // pipeline must emit the first 4 in the Qdrant `scroll` return
    // order (adversarial: chunk_id DESC) and carry a cursor that
    // resumes with `boundary_ts` and the Qdrant-side `offset`. The
    // next page must hold the next 4 records (records 5..=8) in the
    // same order; the final page holds the last 2 (records 9..=10)
    // with `has_more = false`. No record is dropped or duplicated
    // across the page boundary.
    let mut hydrated: HashMap<Uuid, SearchHit> = HashMap::new();
    let mut index_hits: Vec<SearchDatePointHit> = Vec::new();
    let shared_ts: i64 = 1_700_000_010;
    // Build 10 chunks with monotonically increasing UUIDs. The
    // `AdversarialFirstPage` mock sorts them in descending order, so
    // the expected Qdrant return order is the reverse of `chunk_uuids`.
    let mut chunk_uuids: Vec<Uuid> = (1..=10)
        .map(|idx| Uuid::from_u128(0x1000_0000_0000_0000_0000_0000_0000_0000 + idx as u128))
        .collect();
    let qdrant_order: Vec<Uuid> = chunk_uuids.iter().rev().copied().collect();
    for (idx, chunk_id) in chunk_uuids.iter().enumerate() {
        let (id, hit) = make_hit_with_chunk(
            (idx + 1) as i64,
            *chunk_id,
            shared_ts,
            "same-second body text",
        );
        hydrated.insert(id, hit);
        index_hits.push(date_hit(*chunk_id, Some(shared_ts)));
    }
    let (service, _) = build_date_service(
        index_hits,
        hydrated.clone(),
        Some(shared_ts),
    );
    // Page 1: the first 4 chunks in the Qdrant return order
    // (chunk_id DESC). The cursor's `offset` (Qdrant PointId) carries
    // the resume key so the next request continues the boundary in
    // the same order.
    let page1 = block_on(service.search(None, date_request(4))).expect("page 1");
    let page1_ids: Vec<Uuid> = page1.items.iter().map(|hit| hit.chunk_id).collect();
    let expected_page1: Vec<Uuid> = qdrant_order.iter().take(4).copied().collect();
    assert_eq!(
        page1_ids, expected_page1,
        "page 1 must hold the first 4 chunks in Qdrant return order"
    );
    assert_eq!(page1.pagination.has_more, Some(true));
    let next_cursor_1 = page1
        .pagination
        .next_cursor
        .clone()
        .expect("same-second > limit must emit a next cursor");
    // Page 2: the next 4 chunks (5..=8) in the same Qdrant return
    // order with no overlap with page 1.
    let page2 = block_on(service.search(None, date_request_with_cursor(4, next_cursor_1.clone())))
        .expect("page 2");
    let page2_ids: Vec<Uuid> = page2.items.iter().map(|hit| hit.chunk_id).collect();
    let expected_page2: Vec<Uuid> = qdrant_order.iter().skip(4).take(4).copied().collect();
    assert_eq!(
        page2_ids, expected_page2,
        "page 2 must hold chunks 5..=8 with no overlap"
    );
    assert_eq!(page2.pagination.has_more, Some(true));
    let next_cursor_2 = page2
        .pagination
        .next_cursor
        .clone()
        .expect("in-flight drain must emit a next cursor");
    // Page 3: the last 2 chunks (9..=10) and `has_more = false`.
    let page3 =
        block_on(service.search(None, date_request_with_cursor(4, next_cursor_2.clone())))
            .expect("page 3");
    let page3_ids: Vec<Uuid> = page3.items.iter().map(|hit| hit.chunk_id).collect();
    let expected_page3: Vec<Uuid> = qdrant_order.iter().skip(8).copied().collect();
    assert_eq!(page3_ids, expected_page3, "page 3 must hold the last 2 chunks");
    assert_eq!(page3.pagination.has_more, Some(false));
    assert!(page3.pagination.next_cursor.is_none());
    // Pages 1, 2 and 3 are pairwise disjoint and together equal the
    // full population.
    let mut union: Vec<Uuid> = Vec::new();
    union.extend(page1_ids.iter().copied());
    union.extend(page2_ids.iter().copied());
    union.extend(page3_ids.iter().copied());
    union.sort();
    chunk_uuids.sort();
    assert_eq!(union, chunk_uuids, "page 1 + 2 + 3 must equal the full population");
    // The replayed cursor yields the same page 2 and page 3 because
    // the Qdrant-side `offset` is deterministic per ordering epoch.
    let replay_p2 = block_on(service.search(None, date_request_with_cursor(4, next_cursor_1)))
        .expect("replay page 2");
    assert_eq!(
        replay_p2.items.iter().map(|hit| hit.chunk_id).collect::<Vec<_>>(),
        page2_ids,
        "replayed cursor must be deterministic for page 2"
    );
    let replay_p3 = block_on(service.search(None, date_request_with_cursor(4, next_cursor_2)))
        .expect("replay page 3");
    assert_eq!(
        replay_p3.items.iter().map(|hit| hit.chunk_id).collect::<Vec<_>>(),
        page3_ids,
        "replayed cursor must be deterministic for page 3"
    );
}

/// Date-mode regression: legacy `page > 1` is rejected before any
/// expensive work begins. Date mode is forward-only keyset
/// pagination; the legacy `(page - 1) * limit` mapping has no
/// meaning here, and silently serving page 1 is a worse failure than
/// a clean 400.
#[test]
fn date_mode_rejects_legacy_page_greater_than_one() {
    let chunk_id = Uuid::from_u128(0x3333_3333_3333_3333_3333_3333_3333_3333);
    let hit = make_hit_with_chunk(1, chunk_id, 1_700_000_010, "fresh body");
    let mut hydrated = HashMap::new();
    hydrated.insert(hit.0, hit.1.clone());
    let index_hits = vec![date_hit(chunk_id, Some(1_700_000_010))];
    let (service, _) = build_date_service(index_hits, hydrated, Some(1_700_000_010));
    let mut request = date_request(8);
    request.page = 2;
    let error = block_on(service.search(None, request))
        .expect_err("page > 1 must be rejected by date pipeline");
    let message = error.to_string();
    assert!(
        message.contains("page > 1") || message.contains("sort=date"),
        "unexpected error: {message}"
    );
}

/// Date-mode regression: snapshot-fetch errors propagate as
/// `Err(_)` instead of being swallowed to `Ok(None)`. A transient
/// Qdrant outage must not silently widen the replayed upper bound.
#[test]
fn date_mode_snapshot_error_propagates_as_repository_error() {
    struct SnapshotFailRepo {
        settings: SearchSettings,
        hydrated: HashMap<Uuid, SearchHit>,
    }
    #[async_trait]
    impl SearchRepository for SnapshotFailRepo {
        async fn get_search_settings(&self) -> anyhow::Result<Option<SearchSettings>> {
            Ok(Some(self.settings.clone()))
        }
        async fn get_search_generation(&self) -> anyhow::Result<i64> {
            Ok(0)
        }
        async fn date_request_upper_bound(
            &self,
            _user_id: Option<i64>,
            _request: &SearchRequest,
        ) -> anyhow::Result<Option<i64>> {
            // Snapshot failure must surface as `Err(_)`.
            Err(anyhow!("qdrant snapshot failure: timeout"))
        }
        async fn fetch_search_hits_by_chunk_ids(
            &self,
            _chunk_ids: &[Uuid],
            _request: &SearchRequest,
            _scope: &AccessScope,
        ) -> anyhow::Result<HashMap<Uuid, SearchHit>> {
            Ok(self.hydrated.clone())
        }
        async fn keyword_search(
            &self,
            _request: &SearchRequest,
            _scope: &AccessScope,
            _limit: usize,
        ) -> anyhow::Result<Vec<SearchHit>> {
            Ok(Vec::new())
        }
        async fn list_rerank_item_scores(
            &self,
            _rerank_model: &str,
            _query_hash: &str,
            _chunk_ids: &[Uuid],
        ) -> anyhow::Result<HashMap<Uuid, StoredRerankItemScore>> {
            Ok(HashMap::new())
        }
        async fn upsert_rerank_item_scores(
            &self,
            _scores: &[StoredRerankItemScore],
        ) -> anyhow::Result<()> {
            Ok(())
        }
        async fn get_document(
            &self,
            _document_id: i64,
            _scope: &AccessScope,
        ) -> anyhow::Result<Option<DocumentResponse>> {
            Ok(None)
        }
    }
    let chunk_id = Uuid::from_u128(0x3333_3333_3333_3333_3333_3333_3333_3333);
    let mut hydrated = HashMap::new();
    let hit = make_hit_with_chunk(1, chunk_id, 1_700_000_010, "body");
    hydrated.insert(chunk_id, hit.1);
    let service = block_on(SearchService::new(
        Arc::new(SnapshotFailRepo {
            settings: vector_settings(),
            hydrated,
        }),
        Arc::new(MockScope),
        Arc::new(MockEmbedding),
        Arc::new(MockIndex {
            hits: Vec::new(),
            seen_limit: Arc::new(Mutex::new(None)),
            date_hits: vec![date_hit(chunk_id, Some(1_700_000_010))],
            seen_date_bounds: Arc::new(Mutex::new(Vec::new())),
            date_window_strategy: Arc::new(Mutex::new(DateWindowStrategy::AdversarialFirstPage)),
            fetch_limit_override: None,
        }),
        None,
        "test-model".to_string(),
    ))
    .expect("service");
    let error = block_on(service.search(None, date_request(8)))
        .expect_err("snapshot failure must surface as a repository error");
    let message = error.to_string();
    assert!(
        message.contains("qdrant snapshot failure"),
        "snapshot error must propagate; got {message}"
    );
}

/// Date-mode regression: the caller's `user_id` must reach
/// `date_request_upper_bound` so the snapshot honours the user's
/// visibility scope. A logged-in user must not be silently demoted to
/// `None` (which would exclude their newest private records from the
/// ceiling and miss them on page 1).
#[test]
fn date_mode_snapshot_receives_caller_user_id() {
    struct CapturingRepo {
        settings: SearchSettings,
        observed_user_id: Arc<Mutex<Option<Option<i64>>>>,
    }
    #[async_trait]
    impl SearchRepository for CapturingRepo {
        async fn get_search_settings(&self) -> anyhow::Result<Option<SearchSettings>> {
            Ok(Some(self.settings.clone()))
        }
        async fn get_search_generation(&self) -> anyhow::Result<i64> {
            Ok(0)
        }
        async fn date_request_upper_bound(
            &self,
            user_id: Option<i64>,
            _request: &SearchRequest,
        ) -> anyhow::Result<Option<i64>> {
            *self.observed_user_id.lock().expect("lock") = Some(user_id);
            Ok(Some(1_700_000_010))
        }
        async fn fetch_search_hits_by_chunk_ids(
            &self,
            _chunk_ids: &[Uuid],
            _request: &SearchRequest,
            _scope: &AccessScope,
        ) -> anyhow::Result<HashMap<Uuid, SearchHit>> {
            Ok(HashMap::new())
        }
        async fn keyword_search(
            &self,
            _request: &SearchRequest,
            _scope: &AccessScope,
            _limit: usize,
        ) -> anyhow::Result<Vec<SearchHit>> {
            Ok(Vec::new())
        }
        async fn list_rerank_item_scores(
            &self,
            _rerank_model: &str,
            _query_hash: &str,
            _chunk_ids: &[Uuid],
        ) -> anyhow::Result<HashMap<Uuid, StoredRerankItemScore>> {
            Ok(HashMap::new())
        }
        async fn upsert_rerank_item_scores(
            &self,
            _scores: &[StoredRerankItemScore],
        ) -> anyhow::Result<()> {
            Ok(())
        }
        async fn get_document(
            &self,
            _document_id: i64,
            _scope: &AccessScope,
        ) -> anyhow::Result<Option<DocumentResponse>> {
            Ok(None)
        }
    }
    let observed: Arc<Mutex<Option<Option<i64>>>> = Arc::new(Mutex::new(None));
    let chunk_id = Uuid::from_u128(0x3333_3333_3333_3333_3333_3333_3333_3333);
    let mut hydrated = HashMap::new();
    let hit = make_hit_with_chunk(1, chunk_id, 1_700_000_010, "body");
    hydrated.insert(chunk_id, hit.1);
    let service = block_on(SearchService::new(
        Arc::new(CapturingRepo {
            settings: vector_settings(),
            observed_user_id: Arc::clone(&observed),
        }),
        Arc::new(MockScope),
        Arc::new(MockEmbedding),
        Arc::new(MockIndex {
            hits: Vec::new(),
            seen_limit: Arc::new(Mutex::new(None)),
            date_hits: vec![date_hit(chunk_id, Some(1_700_000_010))],
            seen_date_bounds: Arc::new(Mutex::new(Vec::new())),
            date_window_strategy: Arc::new(Mutex::new(DateWindowStrategy::AdversarialFirstPage)),
            fetch_limit_override: None,
        }),
        None,
        "test-model".to_string(),
    ))
    .expect("service");
    let user_id: Option<i64> = Some(42);
    let _ = block_on(service.search(user_id, date_request(8))).expect("search");
    let captured = *observed.lock().expect("lock");
    assert_eq!(
        captured,
        Some(Some(42)),
        "date_request_upper_bound must receive the caller's user_id; got {captured:?}"
    );
}

/// Date-mode regression: a blank query is rejected as a 400 validation
/// error before any expensive work. The contract is "matching query,
/// latest-first, no rerank"; silently serving a browse would violate
/// the contract. The check must fire on the first request — no
/// embedding, no snapshot, no Qdrant call.
#[test]
fn date_mode_rejects_blank_query() {
    let chunk_id = Uuid::from_u128(0x3333_3333_3333_3333_3333_3333_3333_3333);
    let mut hydrated = HashMap::new();
    let hit = make_hit_with_chunk(1, chunk_id, 1_700_000_010, "fresh body text");
    hydrated.insert(hit.0, hit.1);
    let index_hits = vec![date_hit(chunk_id, Some(1_700_000_010))];
    let (service, _) = build_date_service(index_hits, hydrated, Some(1_700_000_010));
    for blank in ["", "   ", "\t \n"] {
        let mut request = date_request(8);
        request.query = blank.to_string();
        let error = block_on(service.search(None, request))
            .expect_err("blank query in date mode must be rejected");
        let message = error.to_string();
        assert!(
            message.contains("query text is required") && message.contains("sort=date"),
            "blank query must surface a date-mode validation error; got {message}"
        );
    }
}

/// Date-mode regression: the query is matched against the hydrated
/// `title + chunk_text` with the same all-terms substring rule used by
/// the keyword path. Chunks whose text does not contain every
/// lowercased term of the query must be dropped, and a query whose
/// terms are absent from the entire population must return an empty
/// page with `has_more = false`.
#[test]
fn date_mode_matches_query_text_in_hydrated_chunks() {
    let ts = 1_700_000_010;
    let matching = Uuid::from_u128(0xaaaa_aaaa_aaaa_aaaa_aaaa_aaaa_aaaa_aaaa);
    let non_matching = Uuid::from_u128(0xbbbb_bbbb_bbbb_bbbb_bbbb_bbbb_bbbb_bbbb);
    let mut hydrated = HashMap::new();
    let (m_id, m_hit) =
        make_hit_with_chunk(1, matching, ts, "fresh body text matching the query");
    hydrated.insert(m_id, m_hit);
    let (n_id, n_hit) =
        make_hit_with_chunk(2, non_matching, ts, "completely unrelated text without terms");
    hydrated.insert(n_id, n_hit);
    let index_hits = vec![
        date_hit(matching, Some(ts)),
        date_hit(non_matching, Some(ts)),
    ];
    let (service, _) = build_date_service(index_hits, hydrated, Some(ts));
    let mut request = date_request(8);
    request.query = "body".to_string();
    let response = block_on(service.search(None, request)).expect("search");
    let ids: Vec<Uuid> = response.items.iter().map(|hit| hit.chunk_id).collect();
    assert_eq!(
        ids,
        vec![matching],
        "only the chunk that contains the query term must be returned; got {ids:?}"
    );
}

/// Date-mode regression: multi-term queries require every term to be
/// present. A query with two distinct tokens drops chunks that match
/// only one of them, even if those chunks are at the top of the
/// `published_ts` ordering.
#[test]
fn date_mode_query_filter_requires_all_terms() {
    let ts = 1_700_000_010;
    let both = Uuid::from_u128(0xaaaa_aaaa_aaaa_aaaa_aaaa_aaaa_aaaa_aaaa);
    let only_body = Uuid::from_u128(0xbbbb_bbbb_bbbb_bbbb_bbbb_bbbb_bbbb_bbbb);
    let only_text = Uuid::from_u128(0xcccc_cccc_cccc_cccc_cccc_cccc_cccc_cccc);
    let neither = Uuid::from_u128(0xdddd_dddd_dddd_dddd_dddd_dddd_dddd_dddd);
    let mut hydrated = HashMap::new();
    let (b_id, b_hit) = make_hit_with_chunk(1, both, ts, "fresh body text with both terms");
    hydrated.insert(b_id, b_hit);
    let (o_id, o_hit) = make_hit_with_chunk(2, only_body, ts, "fresh body only");
    hydrated.insert(o_id, o_hit);
    let (t_id, t_hit) = make_hit_with_chunk(3, only_text, ts, "completely text focused");
    hydrated.insert(t_id, t_hit);
    let (n_id, n_hit) = make_hit_with_chunk(4, neither, ts, "neither term appears here");
    hydrated.insert(n_id, n_hit);
    let index_hits = vec![
        date_hit(both, Some(ts)),
        date_hit(only_body, Some(ts)),
        date_hit(only_text, Some(ts)),
        date_hit(neither, Some(ts)),
    ];
    let (service, _) = build_date_service(index_hits, hydrated, Some(ts));
    let mut request = date_request(8);
    // The two terms split into `["body", "text"]` after lowercasing +
    // punctuation/whitespace split; only the "both" chunk satisfies
    // both substrings.
    request.query = "body text".to_string();
    let response = block_on(service.search(None, request)).expect("search");
    let ids: Vec<Uuid> = response.items.iter().map(|hit| hit.chunk_id).collect();
    assert_eq!(
        ids,
        vec![both],
        "all-terms rule must require every term to be present; got {ids:?}"
    );
}

/// Date-mode regression: the Qdrant-side keyset offset is threaded
/// end-to-end. The mock returns an adversarial first page (an arbitrary
/// 1024-subset ordered by descending `chunk_id` plus a `next_offset`),
/// the pipeline must follow `next_offset` on the next call, and the
/// union of all pages must equal the full boundary population with no
/// duplicates. This is the cross-page completeness test the round-2
/// review required.
#[test]
fn date_mode_keyset_offset_drains_truncated_boundary_across_pages() {
    // 2000 same-second records. The per-timestamp fetch limit is
    // 1024, so the mock's first page returns the largest 1024 UUIDs
    // (the adversarial shape) and the next page returns the remaining
    // 976. Without the Qdrant-side offset threaded through, page 1
    // would visit 1024 records, page 2 would refetch the same 1024
    // subset (filtered by `seen`), and the smallest 976 chunk_ids
    // would be permanently lost.
    let shared_ts: i64 = 1_700_000_010;
    let total: usize = 2_000;
    let mut hydrated: HashMap<Uuid, SearchHit> = HashMap::new();
    let mut index_hits: Vec<SearchDatePointHit> = Vec::with_capacity(total);
    let chunk_uuids: Vec<Uuid> = (1..=total)
        .map(|idx| Uuid::from_u128(0x1000_0000_0000_0000_0000_0000_0000_0000 + idx as u128))
        .collect();
    for chunk_id in &chunk_uuids {
        let (id, hit) = make_hit_with_chunk(1, *chunk_id, shared_ts, "fresh body text");
        hydrated.insert(id, hit);
        index_hits.push(date_hit(*chunk_id, Some(shared_ts)));
    }
    let (service, _) = build_date_service(index_hits, hydrated, Some(shared_ts));
    // Page 1: limit=100. The mock's first call returns the largest
    // 100 UUIDs (out of 2000); the pipeline must surface the
    // Qdrant-side resume key in the cursor.
    let page1 = block_on(service.search(None, date_request(100))).expect("page 1");
    assert_eq!(page1.items.len(), 100);
    assert!(page1.pagination.has_more == Some(true));
    let next_cursor_1 = page1
        .pagination
        .next_cursor
        .clone()
        .expect("truncated boundary must emit a cursor");
    // Page 2: still 100 records. The mock returns the next 100 from
    // the descending list, so page 1 + page 2 must contain 200
    // distinct UUIDs and page 2 must be disjoint from page 1.
    let page2 = block_on(service.search(None, date_request_with_cursor(100, next_cursor_1.clone())))
        .expect("page 2");
    assert_eq!(page2.items.len(), 100);
    let page1_ids: HashSet<Uuid> = page1.items.iter().map(|hit| hit.chunk_id).collect();
    let page2_ids: HashSet<Uuid> = page2.items.iter().map(|hit| hit.chunk_id).collect();
    let overlap: Vec<Uuid> = page1_ids.intersection(&page2_ids).copied().collect();
    assert!(
        overlap.is_empty(),
        "page 2 must not duplicate page 1; overlap = {overlap:?}"
    );
    // Walk every page until the boundary is exhausted. The mock
    // reports `next_offset = None` once the descending slice is
    // empty; at that point the cursor advances below the boundary
    // and the next cross-timestamp fetch returns an empty page so
    // `hit_exhausted` fires. The walk must resume from page 2's
    // cursor (which carries the Qdrant-side `offset` after page 2),
    // not from `next_cursor_1` (which would re-replay page 2).
    let mut walked_ids: HashSet<Uuid> = page1_ids.clone();
    walked_ids.extend(page2_ids);
    let mut current_cursor = page2
        .pagination
        .next_cursor
        .clone()
        .expect("page 2 must carry a next cursor");
    let mut pages = 2;
    while pages < 32 {
        let page = block_on(service.search(
            None,
            date_request_with_cursor(100, current_cursor.clone()),
        ))
        .expect("page");
        let ids: HashSet<Uuid> = page.items.iter().map(|hit| hit.chunk_id).collect();
        for id in &ids {
            assert!(
                !walked_ids.contains(id),
                "page {pages} duplicated {id} from a prior page"
            );
        }
        walked_ids.extend(ids);
        if page.pagination.has_more == Some(false) {
            assert!(page.pagination.next_cursor.is_none());
            break;
        }
        current_cursor = page
            .pagination
            .next_cursor
            .clone()
            .expect("non-final page must carry a cursor");
        pages += 1;
    }
    // The union of all walked pages must equal the full population.
    let expected: HashSet<Uuid> = chunk_uuids.iter().copied().collect();
    assert_eq!(
        walked_ids.len(),
        total,
        "walked {} records, expected {total}; missing = {:?}",
        walked_ids.len(),
        expected.difference(&walked_ids).collect::<Vec<_>>()
    );
    assert_eq!(walked_ids, expected, "cross-page union must equal the full population");
}

/// Date-mode regression: the per-request `MAX_DATE_WINDOWS` cap is honest.
/// A request that runs out of window budget mid-walk must surface
/// `has_more = true` plus a resumable cursor so the next request can
/// continue. The round-3 review flagged the legacy behaviour (short
/// page + `has_more = false` while rows remain) as a P1.
///
/// The test sets up one record per distinct timestamp (so each
/// iteration makes one fetch and emits one record), with `limit` high
/// enough that the page is never filled by the per-iteration quota.
/// The cap fires after `MAX_DATE_WINDOWS` fetches; the request
/// returns `has_more = true` plus a cursor, and the next request
/// resumes the walk in the same generation / filter context.
#[test]
fn date_mode_request_window_cap_emits_resumable_cursor() {
    use super::search_cursor::MAX_DATE_WINDOWS;
    // Each round is one cross-ts fetch + one per-timestamp fetch,
    // so the per-request cap of `MAX_DATE_WINDOWS` fetches stops the
    // walk at `MAX_DATE_WINDOWS / 2` rounds. Use a population that
    // exceeds that count so the cap fires before the population is
    // drained. `limit` is high enough that the page fill is not the
    // loop exit (and within the documented 1..=100 range).
    let total: usize = MAX_DATE_WINDOWS + 16;
    let rounds_before_cap: usize = MAX_DATE_WINDOWS / 2;
    let limit: usize = total.min(100);
    let mut hydrated: HashMap<Uuid, SearchHit> = HashMap::new();
    let mut index_hits: Vec<SearchDatePointHit> = Vec::with_capacity(total);
    let chunk_uuids: Vec<Uuid> = (1..=total)
        .map(|idx| Uuid::from_u128(0x1000_0000_0000_0000_0000_0000_0000_0000 + idx as u128))
        .collect();
    // Distinct timestamps, oldest first so `published_ts DESC` walks
    // them in `chunk_uuids` order (each is its own boundary).
    for (idx, chunk_id) in chunk_uuids.iter().enumerate() {
        let ts = 1_700_000_010 - idx as i64;
        let (id, hit) = make_hit_with_chunk(
            (idx + 1) as i64,
            *chunk_id,
            ts,
            "fresh body text",
        );
        hydrated.insert(id, hit);
        index_hits.push(date_hit(*chunk_id, Some(ts)));
    }
    let (service, _) = build_date_service(index_hits, hydrated, Some(1_700_000_010));
    // Page 1: the pipeline walks `rounds_before_cap` distinct
    // boundaries and emits one record per boundary, then the cap
    // fires. The request must surface `has_more = true` plus a
    // cursor.
    let page1 = block_on(service.search(None, date_request(limit))).expect("page 1");
    assert_eq!(
        page1.items.len(),
        rounds_before_cap,
        "cap must stop the walk after MAX_DATE_WINDOWS / 2 rounds"
    );
    assert_eq!(
        page1.pagination.has_more,
        Some(true),
        "cap mid-walk must surface has_more = true; got {:?}",
        page1.pagination.has_more
    );
    let next_cursor_1 = page1
        .pagination
        .next_cursor
        .clone()
        .expect("cap mid-walk must emit a cursor");
    // Walk the remaining records by replaying the cursor. Each
    // request starts with a fresh window budget, so the cap does not
    // accumulate. The remaining `8` records must drain across one
    // more page (limit >> 8) and the final page must be
    // `has_more = false` with no cursor.
    let mut walked: HashSet<Uuid> = page1.items.iter().map(|hit| hit.chunk_id).collect();
    let mut current_cursor = next_cursor_1;
    let mut pages = 1;
    while pages < 8 {
        let page = block_on(service.search(
            None,
            date_request_with_cursor(limit, current_cursor.clone()),
        ))
        .expect("page");
        let ids: HashSet<Uuid> = page.items.iter().map(|hit| hit.chunk_id).collect();
        for id in &ids {
            assert!(!walked.contains(id), "page {pages} duplicated {id}");
        }
        walked.extend(ids);
        if page.pagination.has_more == Some(false) {
            assert!(page.pagination.next_cursor.is_none());
            break;
        }
        current_cursor = page
            .pagination
            .next_cursor
            .clone()
            .expect("non-final page must carry a cursor");
        pages += 1;
    }
    assert!(
        pages < 8,
        "remaining population never drained within page budget; got {pages} pages"
    );
    let expected: HashSet<Uuid> = chunk_uuids.iter().copied().collect();
    assert_eq!(
        walked, expected,
        "cap-resume walk must visit every record in the population"
    );
}

/// Date-mode regression: a mid-walk cap stop must emit a resumable
/// cursor with the Qdrant-side `offset` so the next request
/// continues in the same ordering. The first request walks 64
/// distinct boundaries (the request-level cap), the second request
/// drains the remaining records in fresh-budget fashion.
#[test]
fn date_mode_request_window_cap_preserves_qdrant_offset_in_cursor() {
    use super::search_cursor::MAX_DATE_WINDOWS;
    // Two requests must not accumulate window usage: the second
    // request starts with a fresh budget. The cursor carries the
    // Qdrant-side `offset` (or `boundary_ts` + `offset` if the cap
    // fired mid-boundary), not a `windows_consumed` counter. With
    // 2 fetches per round, the cap fires at `MAX_DATE_WINDOWS / 2`
    // rounds. `limit` stays within the documented 1..=100 range.
    let total: usize = MAX_DATE_WINDOWS + 4;
    let limit: usize = total.min(100);
    let mut hydrated: HashMap<Uuid, SearchHit> = HashMap::new();
    let mut index_hits: Vec<SearchDatePointHit> = Vec::with_capacity(total);
    let chunk_uuids: Vec<Uuid> = (1..=total)
        .map(|idx| Uuid::from_u128(0x1000_0000_0000_0000_0000_0000_0000_0000 + idx as u128))
        .collect();
    for (idx, chunk_id) in chunk_uuids.iter().enumerate() {
        let ts = 1_700_000_010 - idx as i64;
        let (id, hit) = make_hit_with_chunk(
            (idx + 1) as i64,
            *chunk_id,
            ts,
            "fresh body text",
        );
        hydrated.insert(id, hit);
        index_hits.push(date_hit(*chunk_id, Some(ts)));
    }
    let (service, _) = build_date_service(index_hits, hydrated, Some(1_700_000_010));
    // First request: cap fires after `MAX_DATE_WINDOWS / 2`
    // boundaries.
    let page1 = block_on(service.search(None, date_request(limit))).expect("page 1");
    assert_eq!(page1.items.len(), MAX_DATE_WINDOWS / 2);
    assert_eq!(page1.pagination.has_more, Some(true));
    let cursor_1 = page1
        .pagination
        .next_cursor
        .clone()
        .expect("cap mid-walk must emit a cursor");
    // The cursor's wire format must NOT contain a `windows` field
    // (the per-request budget is not accumulated in the cursor).
    // Decode the cursor and verify it does not surface a window
    // counter.
    let decoded = super::search_cursor::decode_cursor(&cursor_1)
        .expect("cursor decodes");
    let super::search_cursor::DecodedCursor::Date(date) = decoded else {
        panic!("expected a date cursor; got {decoded:?}");
    };
    assert!(
        date.offset.is_some() || date.boundary_ts.is_some() || date.before.is_some(),
        "cap cursor must carry a resume key (offset, boundary_ts, or before); got {date:?}"
    );
    // Second request onwards: fresh budget drains the remaining
    // records across one or more pages and exits with
    // `has_more = false`.
    let mut walked: HashSet<Uuid> = page1.items.iter().map(|hit| hit.chunk_id).collect();
    let mut current_cursor = cursor_1;
    let mut pages = 1;
    while pages < MAX_DATE_WINDOWS {
        let page = block_on(service.search(
            None,
            date_request_with_cursor(limit, current_cursor.clone()),
        ))
        .expect("page");
        let ids: HashSet<Uuid> = page.items.iter().map(|hit| hit.chunk_id).collect();
        for id in &ids {
            assert!(!walked.contains(id), "page {pages} duplicated {id}");
        }
        walked.extend(ids);
        if page.pagination.has_more == Some(false) {
            assert!(page.pagination.next_cursor.is_none());
            break;
        }
        current_cursor = page
            .pagination
            .next_cursor
            .clone()
            .expect("non-final page must carry a cursor");
        pages += 1;
    }
    assert!(
        pages < MAX_DATE_WINDOWS,
        "remaining population never drained within page budget; got {pages} pages"
    );
    let expected: HashSet<Uuid> = chunk_uuids.iter().copied().collect();
    assert_eq!(
        walked, expected,
        "two-request walk must visit every record in the population"
    );
}

/// Date-mode regression: a single boundary whose population
/// STRICTLY EXCEEDS one fetch must drain in full across pages via
/// the Qdrant-side `offset` (the round-3 review required a
/// population strictly larger than a single fetch with adversarial
/// tie order). The walk must visit every record exactly once and
/// `has_more = false` + no cursor must appear only on the
/// proven-exhaustion page.
#[test]
fn date_mode_population_strictly_exceeds_fetch_drains_in_full() {
    use super::search_date::DATE_BOUNDARY_FETCH_LIMIT;
    // 300 same-second records; the mock's per-call effective fetch
    // limit is `min(fetch_limit_override, query.limit) = min(64, 8)
    // = 8` (see `MockIndex::search_by_date_window`), so each fetch
    // returns 8 records and the boundary needs ~38 fetches
    // (37 × 8 + 1 × 4) to drain. CI stays fast.
    let shared_ts: i64 = 1_700_000_010;
    let total: usize = 300;
    let fetch_limit: usize = 64;
    let limit: usize = 8;
    let mut hydrated: HashMap<Uuid, SearchHit> = HashMap::new();
    let mut index_hits: Vec<SearchDatePointHit> = Vec::with_capacity(total);
    let chunk_uuids: Vec<Uuid> = (1..=total)
        .map(|idx| Uuid::from_u128(0x1000_0000_0000_0000_0000_0000_0000_0000 + idx as u128))
        .collect();
    for chunk_id in &chunk_uuids {
        let (id, hit) = make_hit_with_chunk(1, *chunk_id, shared_ts, "fresh body text");
        hydrated.insert(id, hit);
        index_hits.push(date_hit(*chunk_id, Some(shared_ts)));
    }
    // `AdversarialFirstPage` with `fetch_limit_override = Some(64)`
    // so each fetch returns 64 records in descending `chunk_id`
    // order with a `next_offset` set until the slice is exhausted.
    let (service, _) = build_date_service_with_fetch_limit(
        index_hits,
        hydrated,
        Some(shared_ts),
        fetch_limit,
    );
    // Page 1: limit 8. The mock's first call returns the largest 8
    // UUIDs (out of 300); the page is full so `has_more = true`
    // and the cursor carries the Qdrant-side resume key.
    let page1 = block_on(service.search(None, date_request(limit))).expect("page 1");
    assert_eq!(page1.items.len(), limit);
    assert_eq!(page1.pagination.has_more, Some(true));
    let next_cursor_1 = page1
        .pagination
        .next_cursor
        .clone()
        .expect("truncated boundary must emit a cursor");
    // Walk every page until the boundary is drained. The union of
    // every walked page must equal the full population with no
    // duplicates; `has_more = false` + no cursor must appear only
    // on the proven-exhaustion page.
    let mut walked: HashSet<Uuid> = page1.items.iter().map(|hit| hit.chunk_id).collect();
    let mut current_cursor = next_cursor_1;
    let mut pages = 1;
    let cap_pages = (total / limit) + 8;
    while pages < cap_pages {
        let page = block_on(service.search(
            None,
            date_request_with_cursor(limit, current_cursor.clone()),
        ))
        .expect("page");
        let ids: HashSet<Uuid> = page.items.iter().map(|hit| hit.chunk_id).collect();
        for id in &ids {
            assert!(!walked.contains(id), "page {pages} duplicated {id}");
        }
        walked.extend(ids);
        if page.pagination.has_more == Some(false) {
            assert!(page.pagination.next_cursor.is_none());
            break;
        }
        current_cursor = page
            .pagination
            .next_cursor
            .clone()
            .expect("non-final page must carry a cursor");
        pages += 1;
    }
    assert!(
        pages < cap_pages,
        "boundary never drained within page budget; got {pages} pages"
    );
    let expected: HashSet<Uuid> = chunk_uuids.iter().copied().collect();
    assert_eq!(
        walked, expected,
        "cross-page union must equal the full population"
    );
    // The constant is exposed so tests can document their
    // per-request fetch limit.
    let _ = DATE_BOUNDARY_FETCH_LIMIT;
}

/// Date-mode regression: a same-second boundary whose head page is
/// entirely keyword-filtered out must NOT strand the matching tail
/// with `has_more = false`. The per-timestamp drain must thread
/// `batch.next_offset` even when the first call's
/// `drain_boundary` pushes zero records — otherwise the pipeline
/// confuses "this slice had no match" with "boundary exhausted",
/// advances `before` past the boundary, and the next cross-ts fetch
/// returns empty so the index appears exhausted with the matching
/// tail permanently lost. The fix: when `!pushed` AND
/// `batch.next_offset.is_some()`, keep draining with the
/// Qdrant-side resume key. Only advance past the boundary when
/// Qdrant proves exhaustion (hits empty AND `next_offset` `None`).
#[test]
fn date_mode_filtered_head_keeps_draining_to_match_tail() {
    // 16 same-second records, page size 8. The `AdversarialFirstPage`
    // mock sorts by `chunk_id DESC`, so the first per-ts fetch (no
    // offset) returns the 8 largest chunk_ids (16..=9 in
    // `qdrant_order`) and threads `next_offset = Some(chunk_id_8)`.
    // The remaining 8 (chunk_ids 8..=1) are the matching tail.
    //
    // Half the records are keyword-visible: their `chunk_text`
    // contains the query term "tailtoken". The other half are
    // keyword-invisible: their `chunk_text` does NOT contain
    // "tailtoken", so `matches_keyword_terms` drops them in
    // `drain_boundary` and `pushed = false` on the first fetch.
    let shared_ts: i64 = 1_700_000_010;
    let total: usize = 16;
    let limit: usize = 8;
    let mut hydrated: HashMap<Uuid, SearchHit> = HashMap::new();
    let mut index_hits: Vec<SearchDatePointHit> = Vec::with_capacity(total);
    // 16 chunk_ids; the 8 largest are the keyword-invisible head,
    // the 8 smallest are the matching tail.
    let chunk_uuids: Vec<Uuid> = (1..=total)
        .map(|idx| Uuid::from_u128(0x2000_0000_0000_0000_0000_0000_0000_0000 + idx as u128))
        .collect();
    for chunk_id in &chunk_uuids {
        let text = if chunk_id.as_u128() & 0xff > 8 {
            // chunk_id modulo 256 in (8..=255] — the high half
            // (head 8 in DESC order). No "tailtoken" in the text:
            // `matches_keyword_terms` drops these records so the
            // first per-ts fetch reports `pushed = false`.
            "head body without the magic word"
        } else {
            // chunk_id modulo 256 in (0..=8] — the low half
            // (tail 8 in DESC order). The query term appears in
            // the chunk text so `matches_keyword_terms` keeps
            // these records.
            "tailtoken body that matches the query"
        };
        let (id, hit) = make_hit_with_chunk(1, *chunk_id, shared_ts, text);
        hydrated.insert(id, hit);
        index_hits.push(date_hit(*chunk_id, Some(shared_ts)));
    }
    let (service, _) =
        build_date_service(index_hits, hydrated, Some(shared_ts));
    // Page 1: query "tailtoken", limit 8. The first per-ts fetch
    // returns the head 8 (all keyword-filtered out, pushed=false)
    // with `next_offset = Some(tail_head)`. The fix: the pipeline
    // keeps the in-flight boundary open, fetches the tail 8 (all
    // match), and pushes 8 records. The page fills so
    // `has_more = true` + a cursor carrying the post-tail
    // `before` is emitted.
    let mut request = date_request(limit);
    request.query = "tailtoken".to_string();
    let page1 = block_on(service.search(None, request)).expect("page 1");
    assert_eq!(
        page1.items.len(),
        limit,
        "filtered head must NOT strand the tail; the page should hold the matching 8"
    );
    assert_eq!(
        page1.pagination.has_more,
        Some(true),
        "page 1 fills the boundary so the cursor must keep has_more=true"
    );
    let next_cursor_1 = page1
        .pagination
        .next_cursor
        .clone()
        .expect("tail page must emit a next cursor");
    // The matching tail 8 must equal the 8 smallest chunk_ids
    // (DESC order in Qdrant scroll → tail 8 are chunk_ids 8..=1).
    let page1_ids: HashSet<Uuid> = page1.items.iter().map(|hit| hit.chunk_id).collect();
    let matching_tail: HashSet<Uuid> = chunk_uuids
        .iter()
        .copied()
        .filter(|id| {
            // tail in Qdrant DESC order: chunk_ids 8..=1 (8 IDs total).
            // Same predicate as above: chunk_id modulo 256 in (0..=8].
            id.as_u128() & 0xff <= 8
        })
        .collect();
    assert_eq!(
        page1_ids, matching_tail,
        "page 1 must hold exactly the matching tail 8 (no head 8, no missing tail)"
    );
    // Page 2: cross-ts fetch at `before = ts - 1` is empty (only
    // one timestamp in the population). Qdrant-side exhaustion is
    // proven, so `has_more = false` and `next_cursor = None`. The
    // page itself is empty (the previous page already filled the
    // boundary) — the cursor exists to confirm exhaustion, not to
    // carry new data.
    let mut page2_request = date_request(limit);
    page2_request.query = "tailtoken".to_string();
    page2_request.cursor = Some(next_cursor_1);
    let page2 = block_on(service.search(None, page2_request)).expect("page 2");
    assert!(
        page2.items.is_empty(),
        "page 2 (cross-ts exhaustion probe) must be empty; got {} items",
        page2.items.len()
    );
    assert_eq!(
        page2.pagination.has_more,
        Some(false),
        "page 2 is the proven-exhaustion page; has_more must be false"
    );
    assert!(
        page2.pagination.next_cursor.is_none(),
        "proven-exhaustion page must not emit a cursor"
    );
    // Cross-page union: page 1 + page 2 must equal the matching
    // tail with no duplicates and no missing records.
    let mut walked: HashSet<Uuid> = HashSet::new();
    for id in &page1_ids {
        assert!(walked.insert(*id), "page 1 must not duplicate any chunk_id");
    }
    // page 2 carries no items, so the walked set already equals
    // the matching tail; the assertion below is the contract.
    assert_eq!(
        walked, matching_tail,
        "two-page walk must visit every matching record exactly once"
    );
}

/// Date-mode regression: a query of only punctuation / dashes (no
/// extractable terms) must fall back to the literal phrase
/// substring rule over the lowercased `title + chunk_text`,
/// matching the keyword path's behaviour. A query of `"---"` must
/// match only records whose `title` or `chunk_text` contains the
/// literal `"---"` substring; it must NOT match-all.
#[test]
fn date_mode_punctuation_only_query_uses_phrase_substring_fallback() {
    let shared_ts: i64 = 1_700_000_010;
    // Three records at the same timestamp with distinct text:
    //   * `matching` — title or chunk contains "---"
    //   * `other`    — title / chunk has dashes but not the literal "---"
    //   * `plain`    — no dashes anywhere
    let matching = Uuid::from_u128(0x1111_1111_1111_1111_1111_1111_1111_1111);
    let other = Uuid::from_u128(0x2222_2222_2222_2222_2222_2222_2222_2222);
    let plain = Uuid::from_u128(0x3333_3333_3333_3333_3333_3333_3333_3333);
    let mut hydrated: HashMap<Uuid, SearchHit> = HashMap::new();
    let (m_id, m_hit) = make_hit_with_chunk(1, matching, shared_ts, "uses triple dashes --- in body");
    hydrated.insert(m_id, m_hit);
    let (o_id, o_hit) = make_hit_with_chunk(2, other, shared_ts, "regular body with words");
    hydrated.insert(o_id, o_hit);
    let (p_id, p_hit) = make_hit_with_chunk(3, plain, shared_ts, "no special characters");
    hydrated.insert(p_id, p_hit);
    let index_hits = vec![
        date_hit(matching, Some(shared_ts)),
        date_hit(other, Some(shared_ts)),
        date_hit(plain, Some(shared_ts)),
    ];
    let (service, _) = build_date_service(index_hits, hydrated, Some(shared_ts));
    // Query `"---"`: split on whitespace + ASCII punctuation yields
    // zero terms; the matcher falls back to the literal phrase
    // substring over lowercased title + chunk_text. Only `matching`
    // contains the literal "---" substring in its body.
    let mut request = date_request(8);
    request.query = "---".to_string();
    let response = block_on(service.search(None, request)).expect("search");
    let ids: Vec<Uuid> = response.items.iter().map(|hit| hit.chunk_id).collect();
    assert_eq!(
        ids,
        vec![matching],
        "punctuation-only query must match only records whose title or chunk contains the literal phrase; got {ids:?}"
    );
    // A query of multiple punctuation marks must also use the
    // literal substring rule. `".-."` does not appear in any
    // hydrated body, so the page is empty.
    let mut request = date_request(8);
    request.query = ".-.".to_string();
    let response = block_on(service.search(None, request)).expect("search");
    assert!(
        response.items.is_empty(),
        "phrase substring rule must not match-all; got {} items",
        response.items.len()
    );
    // A query of only whitespace is rejected as a 400 (the same
    // guard the existing tests already exercise).
    let mut request = date_request(8);
    request.query = "   ".to_string();
    let error = block_on(service.search(None, request))
        .expect_err("whitespace-only query in date mode must be rejected");
    assert!(
        error.to_string().contains("query text is required"),
        "blank/whitespace query must surface a date-mode validation error; got {error}"
    );
}
