use std::{sync::Arc, time::Instant};

use anyhow::{Context, Result};
use context69_contracts::{
    DocumentResponse, Pagination, SearchMode, SearchRequest, SearchResponse,
};
use serde_json::Value;
use tracing::{info, warn};

use crate::ranking::{
    apply_rerank, compare_hits, merge_cached_item_scores, merge_candidates, rerank_document_text,
};
use crate::{
    AccessScope, CachedRerankItemScore, OpenRouterRerankClient, RerankDocument, SearchCache,
    SearchEmbeddingProvider, SearchIndex, SearchRepository, SearchScopeResolver, SearchSettings,
    StoredRerankItemScore, rerank_hits_from_item_scores, rerank_item_scores_complete,
    rerank_item_scores_from_hits, sort_cached_item_scores,
};

#[cfg(test)]
#[path = "service_tests.rs"]
mod service_tests;

/// Fixed upper bound for the server-side candidate window.
///
/// Qdrant has no offset and keyword SQL has no OFFSET; the service pages within
/// this bounded window and reports a lower-bound `total`, never an exact count.
pub(crate) const MAX_SEARCH_CANDIDATE_WINDOW: usize = 2_000;

/// Compute the fetch size that preserves one probe item beyond the window.
///
/// Returns `(fetch_limit, can_probe)`. When the window already reaches the fixed
/// cap, no safe probe exists and `can_probe` is `false`.
pub(crate) fn probe_fetch_limit(requested_limit: usize) -> (usize, bool) {
    if requested_limit < MAX_SEARCH_CANDIDATE_WINDOW {
        match requested_limit.checked_add(1) {
            Some(next) if next <= MAX_SEARCH_CANDIDATE_WINDOW => (next, true),
            _ => (requested_limit, false),
        }
    } else {
        (requested_limit, false)
    }
}

/// Resolve lower-bound total and `has_more` from the collected window.
///
/// `collected_len` is the window length before `skip(offset).take(limit)`;
/// `requested_limit` is `offset + limit`. Never reports `Some(false)` when
/// probing was impossible or an upstream fetch reached its requested top-K
/// limit (including limits below the fixed cap with heavy filtering).
pub(crate) fn resolve_search_window(
    collected_len: usize,
    requested_limit: usize,
    can_probe: bool,
    upstream_capped: bool,
) -> (usize, Option<bool>) {
    if collected_len > requested_limit {
        let floor = requested_limit.saturating_add(1);
        (collected_len.max(floor), Some(true))
    } else if !can_probe || upstream_capped {
        (collected_len, None)
    } else {
        (collected_len, Some(false))
    }
}

#[derive(Clone)]
pub struct SearchService {
    repository: Arc<dyn SearchRepository>,
    scope_resolver: Arc<dyn SearchScopeResolver>,
    embedding: Arc<dyn SearchEmbeddingProvider>,
    index: Arc<dyn SearchIndex>,
    rerank: OpenRouterRerankClient,
    cache: SearchCache,
    embedding_model: String,
}

impl SearchService {
    pub async fn new(
        repository: Arc<dyn SearchRepository>,
        scope_resolver: Arc<dyn SearchScopeResolver>,
        embedding: Arc<dyn SearchEmbeddingProvider>,
        index: Arc<dyn SearchIndex>,
        valkey_url: Option<&str>,
        embedding_model: String,
    ) -> Result<Self> {
        Ok(Self {
            repository,
            scope_resolver,
            embedding,
            index,
            rerank: OpenRouterRerankClient::new()?,
            cache: SearchCache::new(valkey_url).await,
            embedding_model,
        })
    }

    pub async fn search(
        &self,
        user_id: Option<i64>,
        request: SearchRequest,
    ) -> Result<SearchResponse> {
        let search_started = Instant::now();
        let page = u32::try_from(request.page)?;
        let page_size = u32::try_from(request.limit)?;
        let offset = usize::try_from(Pagination::offset(page, page_size)?)?;
        let requested_limit = offset
            .checked_add(request.limit)
            .ok_or_else(|| anyhow::anyhow!("search result limit is too large"))?;
        // Probe one item beyond the requested window while staying under the cap.
        let (fetch_limit, can_probe_window) = probe_fetch_limit(requested_limit);
        let scope = self
            .scope_resolver
            .access_scope(user_id, request.group_path.clone())
            .await?;
        let settings = self
            .repository
            .get_search_settings()
            .await?
            .unwrap_or_else(SearchSettings::default);
        let generation = self.repository.get_search_generation().await?;
        let request_hash = SearchCache::request_hash(&request);
        let settings_hash = SearchCache::settings_hash(&settings);
        if let Some(response) = self
            .cache
            .get_search_response(generation, &request_hash, &settings_hash)
            .await
        {
            return Ok(response);
        }

        let query_hash = SearchCache::query_hash(&request.query);
        let vector_multiplier = if request.metadata_filters.is_empty() {
            1
        } else {
            8
        };
        let vector_limit = fetch_limit
            .max(80)
            .saturating_mul(vector_multiplier)
            .min(MAX_SEARCH_CANDIDATE_WINDOW);

        let vector = if let Some(vector) = self
            .cache
            .get_query_embedding(&self.embedding_model, &query_hash)
            .await
        {
            vector
        } else {
            let vector = self.embedding.embed_query(&request.query).await?;
            self.cache
                .set_query_embedding(&self.embedding_model, &query_hash, &vector)
                .await;
            vector
        };
        let mut vector_request = request.clone();
        vector_request.limit = vector_limit;
        let hits = self.index.search(vector, &vector_request, &scope).await?;
        let vector_candidate_count = hits.len();
        // Any saturated top-K may hide deeper candidates, even below the fixed
        // cap when metadata or meaningful-text filtering later drops items.
        let vector_upstream_capped = vector_candidate_count >= vector_limit;
        let mut upstream_capped = vector_upstream_capped;
        let chunk_ids = hits.iter().map(|hit| hit.chunk_id).collect::<Vec<_>>();
        let hydrated = self
            .repository
            .fetch_search_hits_by_chunk_ids(&chunk_ids, &request, &scope)
            .await?;

        let vector_results = hits
            .into_iter()
            .filter_map(|hit| {
                hydrated
                    .get(&hit.chunk_id)
                    .cloned()
                    .map(|mut hydrated_hit| {
                        hydrated_hit.score = hit.score;
                        hydrated_hit.vector_score = Some(hit.score);
                        hydrated_hit
                    })
            })
            .filter(|hit| {
                is_meaningful_text(&hit.chunk_text)
                    && metadata_filters_match(&hit.metadata_json, &request)
            })
            .collect::<Vec<_>>();

        let mut results = if settings.mode == SearchMode::Hybrid {
            let keyword_limit = fetch_limit
                .max(80)
                .saturating_mul(vector_multiplier)
                .min(MAX_SEARCH_CANDIDATE_WINDOW);
            let keyword_hits = self
                .repository
                .keyword_search(&request, &scope, keyword_limit)
                .await?;
            let keyword_candidate_count = keyword_hits.len();
            if keyword_candidate_count >= keyword_limit {
                upstream_capped = true;
            }
            // Filter meaningless text before the probe `take` so a meaningless
            // slot cannot consume the extra item.
            let keyword_results = keyword_hits
                .into_iter()
                .filter(|hit| {
                    is_meaningful_text(&hit.chunk_text)
                        && metadata_filters_match(&hit.metadata_json, &request)
                })
                .collect();
            info!(
                vector_candidate_count,
                keyword_candidate_count,
                metadata_filter_count = request.metadata_filters.len(),
                "search candidates collected"
            );
            let mut candidates = merge_candidates(vector_results, keyword_results, &request.query);
            candidates.sort_by(compare_hits);
            let rerank_limit = settings.candidate_limit.min(candidates.len());
            if settings.rerank_enabled && rerank_limit > 0 {
                let rerank_top_n = fetch_limit.min(rerank_limit);
                let rerank_candidates = candidates
                    .iter()
                    .take(rerank_limit)
                    .cloned()
                    .collect::<Vec<_>>();
                let rerank_chunk_ids = rerank_candidates
                    .iter()
                    .map(|hit| hit.chunk_id)
                    .collect::<Vec<_>>();
                let candidate_hash = SearchCache::candidate_hash(&rerank_chunk_ids);
                if let Some(reranked) = self
                    .cache
                    .get_rerank_batch(
                        generation,
                        &settings.rerank_model,
                        &query_hash,
                        rerank_top_n,
                        &candidate_hash,
                    )
                    .await
                {
                    apply_rerank(candidates, reranked, fetch_limit)
                } else {
                    let mut cached_item_scores = self
                        .cache
                        .get_rerank_item_scores(
                            &settings.rerank_model,
                            &query_hash,
                            &rerank_chunk_ids,
                        )
                        .await;
                    if !rerank_item_scores_complete(&rerank_chunk_ids, &cached_item_scores) {
                        let persisted_scores = self
                            .repository
                            .list_rerank_item_scores(
                                &settings.rerank_model,
                                &query_hash,
                                &rerank_chunk_ids,
                            )
                            .await?
                            .into_values()
                            .map(|score| CachedRerankItemScore {
                                chunk_id: score.chunk_id,
                                score: score.score,
                            })
                            .collect::<Vec<_>>();
                        cached_item_scores =
                            merge_cached_item_scores(cached_item_scores, persisted_scores);
                    }

                    if rerank_item_scores_complete(&rerank_chunk_ids, &cached_item_scores) {
                        let sorted_scores =
                            sort_cached_item_scores(&rerank_chunk_ids, &cached_item_scores);
                        let reranked = rerank_hits_from_item_scores(&sorted_scores);
                        self.cache
                            .set_rerank_batch(
                                generation,
                                &settings.rerank_model,
                                &query_hash,
                                rerank_top_n,
                                &candidate_hash,
                                &reranked,
                            )
                            .await;
                        apply_rerank(candidates, reranked, fetch_limit)
                    } else {
                        let documents = rerank_candidates
                            .iter()
                            .map(|hit| RerankDocument {
                                text: rerank_document_text(hit),
                            })
                            .collect::<Vec<_>>();
                        match self
                            .rerank
                            .rerank(&request.query, &documents, rerank_top_n, &settings)
                            .await
                        {
                            Ok(reranked) => {
                                self.cache
                                    .set_rerank_batch(
                                        generation,
                                        &settings.rerank_model,
                                        &query_hash,
                                        rerank_top_n,
                                        &candidate_hash,
                                        &reranked,
                                    )
                                    .await;
                                let item_scores =
                                    rerank_item_scores_from_hits(&rerank_chunk_ids, &reranked);
                                self.cache
                                    .set_rerank_item_scores(
                                        &settings.rerank_model,
                                        &query_hash,
                                        &item_scores,
                                    )
                                    .await;
                                let persisted = item_scores
                                    .iter()
                                    .map(|score| StoredRerankItemScore {
                                        rerank_model: settings.rerank_model.clone(),
                                        query_hash: query_hash.clone(),
                                        query_text_trimmed: request.query.trim().to_string(),
                                        chunk_id: score.chunk_id,
                                        score: score.score,
                                    })
                                    .collect::<Vec<_>>();
                                if let Err(error) =
                                    self.repository.upsert_rerank_item_scores(&persisted).await
                                {
                                    warn!(error = %error, "failed to persist rerank item scores");
                                }
                                apply_rerank(candidates, reranked, fetch_limit)
                            }
                            Err(error) => {
                                warn!(error = %error, "rerank failed; falling back to local hybrid ranking");
                                candidates.into_iter().take(fetch_limit).collect()
                            }
                        }
                    }
                }
            } else {
                candidates.into_iter().take(fetch_limit).collect()
            }
        } else {
            vector_results.into_iter().take(fetch_limit).collect()
        };

        results.retain(|hit| is_meaningful_text(&hit.chunk_text));

        // Compute `has_more` before slicing the current page so the probe item
        // beyond `offset + limit` is not lost to `take`.
        let collected_len = results.len();
        let (total_window, has_more) = resolve_search_window(
            collected_len,
            requested_limit,
            can_probe_window,
            upstream_capped,
        );
        let total = u64::try_from(total_window)?;
        let items = results
            .into_iter()
            .skip(offset)
            .take(request.limit)
            .collect::<Vec<_>>();
        let response = SearchResponse {
            query: request.query,
            items,
            pagination: Pagination::try_new_search_window(page, page_size, total, has_more)?,
        };
        self.cache
            .set_search_response(generation, &request_hash, &settings_hash, &response)
            .await;
        info!(
            vector_candidate_count,
            result_count = response.items.len(),
            elapsed_ms = search_started.elapsed().as_millis() as u64,
            "search completed"
        );
        Ok(response)
    }

    pub async fn get_document(
        &self,
        document_id: i64,
        scope: &AccessScope,
    ) -> Result<DocumentResponse> {
        self.repository
            .get_document(document_id, scope)
            .await?
            .context("document not found")
    }
}

fn metadata_filters_match(metadata: &Value, request: &SearchRequest) -> bool {
    request.metadata_filters.iter().all(|filter| {
        let found = filter
            .path
            .split('.')
            .try_fold(metadata, |current, segment| {
                current.as_object()?.get(segment)
            });
        match filter.operator {
            context69_contracts::MetadataFilterOperator::Exists => {
                found.is_some_and(|value| !value.is_null())
                    == filter
                        .value
                        .as_ref()
                        .and_then(Value::as_bool)
                        .unwrap_or(true)
            }
            context69_contracts::MetadataFilterOperator::Eq => found == filter.value.as_ref(),
            context69_contracts::MetadataFilterOperator::In => filter
                .value
                .as_ref()
                .and_then(Value::as_array)
                .is_some_and(|values| found.is_some_and(|value| values.contains(value))),
            context69_contracts::MetadataFilterOperator::Contains => {
                found.and_then(Value::as_array).is_some_and(|values| {
                    filter
                        .value
                        .as_ref()
                        .is_some_and(|value| values.contains(value))
                })
            }
            context69_contracts::MetadataFilterOperator::Range => found.is_some_and(|value| {
                let compare = |left: &Value, right: &Value| match (left.as_f64(), right.as_f64()) {
                    (Some(left), Some(right)) => left.partial_cmp(&right),
                    _ => left
                        .as_str()
                        .zip(right.as_str())
                        .map(|(left, right)| left.cmp(right)),
                };
                filter
                    .min
                    .as_ref()
                    .is_none_or(|bound| compare(value, bound).is_some_and(|order| order.is_ge()))
                    && filter.max.as_ref().is_none_or(|bound| {
                        compare(value, bound).is_some_and(|order| order.is_le())
                    })
            }),
        }
    })
}

fn is_meaningful_text(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return false;
    }

    let meaningful = trimmed.chars().filter(|ch| ch.is_alphanumeric()).count();

    meaningful >= 2
}

#[cfg(test)]
mod tests {
    use context69_contracts::Pagination;

    use super::{MAX_SEARCH_CANDIDATE_WINDOW, probe_fetch_limit, resolve_search_window};

    #[test]
    fn probe_adds_one_item_below_cap() {
        let (fetch, can_probe) = probe_fetch_limit(8);
        assert_eq!((fetch, can_probe), (9, true));

        let (fetch, can_probe) = probe_fetch_limit(1_999);
        assert_eq!((fetch, can_probe), (2_000, true));
    }

    #[test]
    fn probe_is_unavailable_at_cap() {
        let (fetch, can_probe) = probe_fetch_limit(MAX_SEARCH_CANDIDATE_WINDOW);
        assert_eq!((fetch, can_probe), (MAX_SEARCH_CANDIDATE_WINDOW, false));

        let (fetch, can_probe) = probe_fetch_limit(MAX_SEARCH_CANDIDATE_WINDOW + 500);
        assert_eq!(
            (fetch, can_probe),
            (MAX_SEARCH_CANDIDATE_WINDOW + 500, false)
        );
    }

    #[test]
    fn first_page_with_extra_candidate_reports_has_more() {
        // page=1, limit=8 => requested=8, probe fetch=9, 9 collected.
        let requested_limit = 8_usize;
        let (fetch, can_probe) = probe_fetch_limit(requested_limit);
        assert!(can_probe);
        assert_eq!(fetch, 9);
        let (total, has_more) = resolve_search_window(9, requested_limit, can_probe, false);
        assert_eq!(has_more, Some(true));
        // Lower-bound total covers offset + limit + 1 for the next page.
        assert!(total >= requested_limit + 1);
        let pagination =
            Pagination::try_new_search_window(1, 8, u64::try_from(total).unwrap(), has_more)
                .unwrap();
        assert_eq!(pagination.has_more, Some(true));
        assert_eq!(pagination.total_is_exact, Some(false));
        assert_eq!(pagination.total, 9);
    }

    #[test]
    fn short_last_page_reports_no_more() {
        // page=2, limit=8 => offset=8, requested=16, only 12 collected.
        let requested_limit = 16_usize;
        let (fetch, can_probe) = probe_fetch_limit(requested_limit);
        assert!(can_probe);
        assert_eq!(fetch, 17);
        let (total, has_more) = resolve_search_window(12, requested_limit, can_probe, false);
        assert_eq!(has_more, Some(false));
        assert_eq!(total, 12);
        // Slicing the last partial page keeps total covering offset + items.
        let offset = 8_usize;
        let items: Vec<usize> = (0..12).skip(offset).take(8).collect();
        assert_eq!(items.len(), 4);
        assert!(total >= offset + items.len());
    }

    #[test]
    fn window_at_cap_reports_unknown_instead_of_false() {
        let requested_limit = MAX_SEARCH_CANDIDATE_WINDOW;
        let (fetch, can_probe) = probe_fetch_limit(requested_limit);
        assert!(!can_probe);
        assert_eq!(fetch, requested_limit);
        let (total, has_more) =
            resolve_search_window(requested_limit, requested_limit, can_probe, false);
        assert_eq!(has_more, None);
        assert_eq!(total, requested_limit);
    }

    #[test]
    fn saturated_upstream_never_reports_false() {
        // Upstream hit the fixed cap but filtering left fewer than requested;
        // claiming `false` would hide candidates beyond the cap.
        let (total, has_more) = resolve_search_window(5, 8, true, true);
        assert_eq!(has_more, None);
        assert_eq!(total, 5);
    }

    #[test]
    fn extra_probe_item_always_wins_over_cap_flags() {
        // An observed extra item proves more results even if a fetch hit the cap.
        let (total, has_more) = resolve_search_window(9, 8, false, true);
        assert_eq!(has_more, Some(true));
        assert!(total >= 9);
    }

    #[test]
    fn search_window_defaults_to_inexact_only_for_search() {
        let legacy = Pagination::try_new(1, 8, 20).unwrap();
        assert_eq!(legacy.has_more, None);
        assert_eq!(legacy.total_is_exact, None);
        // Legacy responses omit the new keys entirely.
        let value = serde_json::to_value(&legacy).unwrap();
        assert!(!value.as_object().unwrap().contains_key("has_more"));
        assert!(!value.as_object().unwrap().contains_key("total_is_exact"));
    }
}
