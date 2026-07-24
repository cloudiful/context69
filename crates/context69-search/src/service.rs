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
        let vector_limit = requested_limit
            .max(80)
            .saturating_mul(vector_multiplier)
            .min(2_000);

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
            let keyword_hits = self
                .repository
                .keyword_search(
                    &request,
                    &scope,
                    requested_limit
                        .max(80)
                        .saturating_mul(vector_multiplier)
                        .min(2_000),
                )
                .await?;
            let keyword_candidate_count = keyword_hits.len();
            let keyword_results = keyword_hits
                .into_iter()
                .filter(|hit| metadata_filters_match(&hit.metadata_json, &request))
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
                let rerank_top_n = requested_limit.min(rerank_limit);
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
                    apply_rerank(candidates, reranked, requested_limit)
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
                        apply_rerank(candidates, reranked, requested_limit)
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
                                apply_rerank(candidates, reranked, requested_limit)
                            }
                            Err(error) => {
                                warn!(error = %error, "rerank failed; falling back to local hybrid ranking");
                                candidates.into_iter().take(requested_limit).collect()
                            }
                        }
                    }
                }
            } else {
                candidates.into_iter().take(requested_limit).collect()
            }
        } else {
            vector_results.into_iter().take(requested_limit).collect()
        };

        results.retain(|hit| is_meaningful_text(&hit.chunk_text));

        let total = results.len();
        let items = results
            .into_iter()
            .skip(offset)
            .take(request.limit)
            .collect::<Vec<_>>();
        let response = SearchResponse {
            query: request.query,
            items,
            pagination: Pagination::try_new(page, page_size, u64::try_from(total)?)?,
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
