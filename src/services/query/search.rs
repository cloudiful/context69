use anyhow::Result;
use tracing::warn;

use super::QueryService;
use super::cache_merge::{
    CachedRerankItemScore, rerank_hits_from_item_scores, rerank_item_scores_complete,
    rerank_item_scores_from_hits, sort_cached_item_scores,
};
use super::ranking::{
    apply_rerank, compare_hits, merge_cached_item_scores, merge_candidates, rerank_document_text,
};
use crate::contracts::{SearchMode, SearchRequest, SearchResponse};
use crate::db::{StoredRerankItemScore, default_search_settings};
use crate::normalize::is_meaningful_text;
use crate::rerank::RerankDocument;
use crate::search_cache::SearchCache;

impl QueryService {
    pub async fn search(
        &self,
        user_id: Option<i64>,
        request: SearchRequest,
    ) -> Result<SearchResponse> {
        let requested_limit = request.limit;
        let scope = self
            .auth
            .access_scope(user_id, request.group_key.clone(), request.project_key.clone())
            .await?;
        let settings = self
            .db
            .get_search_settings()
            .await?
            .unwrap_or_else(default_search_settings);
        let generation = self.db.get_search_generation().await?;
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
        let vector_limit = requested_limit.max(80);

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
        let chunk_ids = hits.iter().map(|hit| hit.chunk_id).collect::<Vec<_>>();
        let hydrated = self
            .db
            .fetch_search_hits_by_chunk_ids(&chunk_ids, &scope)
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
            .filter(|hit| is_meaningful_text(&hit.chunk_text))
            .collect::<Vec<_>>();

        let mut results = if settings.mode == SearchMode::Hybrid {
            let keyword_results = self.db.keyword_search(&request, &scope, 80).await?;
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
                            .db
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
                                    self.db.upsert_rerank_item_scores(&persisted).await
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

        let response = SearchResponse {
            query: request.query,
            hits: results,
        };
        self.cache
            .set_search_response(generation, &request_hash, &settings_hash, &response)
            .await;
        Ok(response)
    }
}
