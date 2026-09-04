use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

use async_trait::async_trait;
use context69_contracts::{DocumentResponse, SearchHit, SearchMode, SearchRequest, Visibility};
use uuid::Uuid;

use crate::{
    AccessScope, SearchEmbeddingProvider, SearchIndex, SearchPointHit, SearchRepository,
    SearchScopeResolver, SearchService, SearchSettings, StoredRerankItemScore,
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
}

struct MockRepo {
    settings: SearchSettings,
    hydrated: HashMap<Uuid, SearchHit>,
    keyword_hits: Vec<SearchHit>,
    seen_keyword_limit: Arc<Mutex<Option<usize>>>,
}

#[async_trait]
impl SearchRepository for MockRepo {
    async fn get_search_settings(&self) -> anyhow::Result<Option<SearchSettings>> {
        Ok(Some(self.settings.clone()))
    }

    async fn get_search_generation(&self) -> anyhow::Result<i64> {
        Ok(0)
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
        metadata_filters: Vec::new(),
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
    };
    let service = block_on(SearchService::new(
        Arc::new(repo),
        Arc::new(MockScope),
        Arc::new(MockEmbedding),
        Arc::new(MockIndex {
            hits: index_hits,
            seen_limit: Arc::clone(&seen_index),
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
