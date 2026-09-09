use std::{sync::Arc, time::Instant};

use anyhow::{Context, Result};
use context69_contracts::{DocumentResponse, SearchHit, SearchMode, SearchRequest, SearchResponse};
use serde_json::Value;
use tracing::{info, warn};

use crate::ranking::{
    FusionWeights, apply_rerank, compare_hits, merge_cached_item_scores, merge_candidates,
    rerank_document_text,
};
use crate::{
    AccessScope, CachedRerankItemScore, RerankClient, RerankDocument, SearchCache,
    SearchEmbeddingProvider, SearchIndex, SearchRepository, SearchScopeResolver, SearchSettings,
    StoredRerankItemScore, rerank_hits_from_item_scores, rerank_item_scores_complete,
    rerank_item_scores_from_hits, sort_cached_item_scores,
};

#[path = "probe.rs"]
mod probe;

#[path = "search_cursor.rs"]
mod search_cursor;

#[path = "search_date.rs"]
mod search_date;

#[path = "search_stream.rs"]
mod search_stream;

#[cfg(test)]
#[path = "service_tests.rs"]
mod service_tests;

pub(crate) use search_date::run_date_search;

use probe::SearchProbe;
use search_cursor::{
    PageWindow, cursor_context, decode_cursor, ensure_cursor_ordering, finalize_window,
    validate_cursor_alignment, validate_cursor_context,
};

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

/// Stage-1 output shared by the POST pipeline and the SSE stream: candidates
/// in local ordering (hybrid merge + local score) or the raw vector ordering.
#[derive(Debug)]
pub(crate) struct CandidateWindow {
    /// Hybrid-merged candidates sorted by local score (hybrid mode) or
    /// meaningful-filtered vector results in vector mode. Not truncated; every
    /// consumer applies `take(fetch_limit)`/rerank itself.
    pub candidates: Vec<SearchHit>,
    /// `true` when the candidates came from the hybrid merge path.
    pub hybrid: bool,
    /// `true` when an upstream fetch saturated its requested top-K.
    pub upstream_capped: bool,
}

/// Stage-2 (rerank) inputs grouped so the pipeline signature stays small.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RerankContext<'a> {
    pub settings: &'a SearchSettings,
    pub request: &'a SearchRequest,
    pub generation: i64,
    pub query_hash: &'a str,
    pub fetch_limit: usize,
}

#[derive(Clone)]
pub struct SearchService {
    repository: Arc<dyn SearchRepository>,
    scope_resolver: Arc<dyn SearchScopeResolver>,
    embedding: Arc<dyn SearchEmbeddingProvider>,
    index: Arc<dyn SearchIndex>,
    rerank: Arc<dyn RerankClient>,
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
            rerank: Arc::new(crate::OpenRouterRerankClient::new()?),
            cache: SearchCache::new(valkey_url).await,
            embedding_model,
        })
    }

    /// Construct a service with a custom rerank client. Tests use this to
    /// inject deterministic mock rerankers; production always wires
    /// `OpenRouterRerankClient` through `new`.
    pub async fn with_rerank_client(
        repository: Arc<dyn SearchRepository>,
        scope_resolver: Arc<dyn SearchScopeResolver>,
        embedding: Arc<dyn SearchEmbeddingProvider>,
        index: Arc<dyn SearchIndex>,
        rerank: Arc<dyn RerankClient>,
        valkey_url: Option<&str>,
        embedding_model: String,
    ) -> Result<Self> {
        Ok(Self {
            repository,
            scope_resolver,
            embedding,
            index,
            rerank,
            cache: SearchCache::new(valkey_url).await,
            embedding_model,
        })
    }

    /// Resolve the absolute candidate offset a request targets: from the opaque
    /// `cursor` when present, otherwise from the deprecated `page` (kept for
    /// compatibility, mapping to the offset `(page - 1) * limit`).
    fn resolved_offset(&self, request: &SearchRequest) -> Result<usize> {
        let page_size =
            u32::try_from(request.limit).map_err(|_| anyhow::anyhow!("page_size is too large"))?;
        if !(1..=100).contains(&page_size) {
            return Err(anyhow::anyhow!("page_size must be between 1 and 100"));
        }
        if let Some(raw_cursor) = request.cursor.as_deref() {
            // Pre-validate the cursor up front so any malformed/oversized
            // payload surfaces as a clean client-facing 400 instead of
            // silently slicing an unrelated offset. The relevance pipeline
            // must reject date cursors (they belong to a different mode).
            let cursor = decode_cursor(raw_cursor)?;
            match cursor {
                search_cursor::DecodedCursor::Relevance(rel) => Ok(rel.offset),
                search_cursor::DecodedCursor::Date(_) => Err(anyhow::anyhow!(
                    "cursor ordering mismatch: the cursor belongs to the date ordering; relevance requests cannot replay it"
                )),
            }
        } else {
            let page =
                u32::try_from(request.page).map_err(|_| anyhow::anyhow!("page is too large"))?;
            usize::try_from(context69_contracts::Pagination::offset(page, page_size)?)
                .map_err(|_| anyhow::anyhow!("page offset is too large"))
        }
    }

    /// Stage 1: embed the query, fetch from the vector index and (in hybrid
    /// mode) the keyword repository concurrently, hydrate the hits, filter
    /// meaningless text and metadata, and produce candidates in local ordering.
    async fn collect_candidates(
        &self,
        request: &SearchRequest,
        scope: &AccessScope,
        settings: &SearchSettings,
        query_hash: &str,
        vector_limit: usize,
        probe: &mut SearchProbe,
    ) -> Result<CandidateWindow> {
        let embed_started = Instant::now();
        let vector = if let Some(vector) = self
            .cache
            .get_query_embedding(&self.embedding_model, query_hash)
            .await
        {
            probe.embed_cache_hit = true;
            vector
        } else {
            let vector = self.embedding.embed_query(&request.query).await?;
            self.cache
                .set_query_embedding(&self.embedding_model, query_hash, &vector)
                .await;
            vector
        };
        probe.embed_elapsed_ms = embed_started.elapsed().as_millis() as u64;
        let mut vector_request = request.clone();
        vector_request.limit = vector_limit;
        // The keyword SQL fetch shares no dependency with the vector search, so
        // in hybrid mode both run concurrently. Each branch times itself so the
        // probe keeps per-segment durations; the two windows overlap on the
        // wall clock. Candidate semantics are unchanged: this only moves when
        // the keyword results arrive, never how they are filtered or merged.
        let (vector_result, keyword_result) = if settings.mode == SearchMode::Hybrid {
            let keyword_limit = vector_limit;
            let (vector, keyword) = tokio::join!(
                async {
                    let started = Instant::now();
                    let result = self.index.search(vector, &vector_request, scope).await;
                    (result, started.elapsed())
                },
                async {
                    let started = Instant::now();
                    let result = self
                        .repository
                        .keyword_search(request, scope, keyword_limit)
                        .await;
                    (result, started.elapsed())
                },
            );
            (vector, Some((keyword, keyword_limit)))
        } else {
            let started = Instant::now();
            let result = self.index.search(vector, &vector_request, scope).await;
            ((result, started.elapsed()), None)
        };
        probe.vector_elapsed_ms = vector_result.1.as_millis() as u64;
        let hits = vector_result.0?;
        let vector_candidate_count = hits.len();
        probe.vector_candidate_count = vector_candidate_count;
        // Any saturated top-K may hide deeper candidates, even below the fixed
        // cap when metadata or meaningful-text filtering later drops items.
        let vector_upstream_capped = vector_candidate_count >= vector_limit;
        let mut upstream_capped = vector_upstream_capped;
        let (keyword_hits, keyword_candidate_count, keyword_capped) = match keyword_result {
            Some(((result, elapsed), keyword_limit)) => {
                probe.keyword_elapsed_ms = elapsed.as_millis() as u64;
                let keyword_hits = result?;
                let count = keyword_hits.len();
                probe.keyword_candidate_count = count;
                probe.keyword_limit = Some(keyword_limit);
                (keyword_hits, count, count >= keyword_limit)
            }
            None => (Vec::new(), 0, false),
        };
        if keyword_capped {
            upstream_capped = true;
        }
        let chunk_ids = hits.iter().map(|hit| hit.chunk_id).collect::<Vec<_>>();
        let hydrate_started = Instant::now();
        let hydrated = self
            .repository
            .fetch_search_hits_by_chunk_ids(&chunk_ids, request, scope)
            .await?;
        probe.hydrate_elapsed_ms = hydrate_started.elapsed().as_millis() as u64;
        probe.hydrate_candidate_count = hydrated.len();

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
                    && metadata_filters_match(&hit.metadata_json, request)
            })
            .collect::<Vec<_>>();

        if settings.mode == SearchMode::Hybrid {
            // Filter meaningless text before the probe `take` so a meaningless
            // slot cannot consume the extra item.
            let keyword_results = keyword_hits
                .into_iter()
                .filter(|hit| {
                    is_meaningful_text(&hit.chunk_text)
                        && metadata_filters_match(&hit.metadata_json, request)
                })
                .collect();
            info!(
                vector_candidate_count,
                keyword_candidate_count,
                metadata_filter_count = request.metadata_filters.len(),
                "search candidates collected"
            );
            let fusion_weights =
                FusionWeights::new(settings.vector_weight, settings.keyword_weight);
            let merge_started = Instant::now();
            let mut candidates = merge_candidates(
                vector_results,
                keyword_results,
                &request.query,
                fusion_weights,
            );
            probe.merge_elapsed_ms = merge_started.elapsed().as_millis() as u64;
            probe.merged_candidate_count = candidates.len();
            candidates.sort_by(compare_hits);
            Ok(CandidateWindow {
                candidates,
                hybrid: true,
                upstream_capped,
            })
        } else {
            Ok(CandidateWindow {
                candidates: vector_results,
                hybrid: false,
                upstream_capped,
            })
        }
    }

    /// Stage 2 (hybrid only): rerank the stable top-N local candidates and
    /// produce the final ordering for the requested window.
    ///
    /// Returns the final-ordered list plus whether rerank ordering was
    /// actually applied. The local-order fallback is used when rerank is
    /// disabled, the candidate set is empty, or the upstream rerank call
    /// fails. The snapshot is the same regardless of the page offset, so
    /// page 1 and page 2 share one rerank cache entry and one consistent
    /// final ordering.
    async fn rerank_stage(
        &self,
        candidates: Vec<SearchHit>,
        ctx: RerankContext<'_>,
        probe: &mut SearchProbe,
    ) -> Result<(Vec<SearchHit>, bool)> {
        let RerankContext {
            settings,
            request,
            generation,
            query_hash,
            fetch_limit,
        } = ctx;
        // Stable snapshot size: bounded by `settings.candidate_limit` (and
        // by the number of candidates we actually collected). Independent of
        // the page offset so the cache key is constant per query epoch.
        let rerank_limit = settings.candidate_limit.min(candidates.len());
        if !settings.rerank_enabled || rerank_limit == 0 {
            return Ok((
                candidates.into_iter().take(fetch_limit).collect(),
                false,
            ));
        }
        let rerank_started = Instant::now();
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
        // `top_n` is the snapshot size, not the page size. Using the snapshot
        // size here makes the cache key independent of `fetch_limit` and
        // guarantees the same external call covers every page of the same
        // query epoch.
        let rerank_top_n = rerank_limit;
        probe.rerank_top_n = rerank_top_n;
        probe.rerank_candidate_len = rerank_candidates.len();
        // Whether rerank ordering was actually applied to the outcome. Cache
        // hits and a successful upstream call reorder the window; a disabled
        // rerank or an upstream failure keep the local ordering.
        let (rerank_outcome, rerank_applied) = if let Some(reranked) = self
            .cache
            .get_rerank_batch(
                generation,
                &settings.rerank_model,
                query_hash,
                rerank_top_n,
                &candidate_hash,
            )
            .await
        {
            probe.rerank_batch_cache_hit = true;
            (
                apply_rerank(candidates, &rerank_candidates, &reranked, fetch_limit),
                true,
            )
        } else {
            let mut cached_item_scores = self
                .cache
                .get_rerank_item_scores(&settings.rerank_model, query_hash, &rerank_chunk_ids)
                .await;
            if !rerank_item_scores_complete(&rerank_chunk_ids, &cached_item_scores) {
                let persisted_scores = self
                    .repository
                    .list_rerank_item_scores(&settings.rerank_model, query_hash, &rerank_chunk_ids)
                    .await?
                    .into_values()
                    .map(|score| CachedRerankItemScore {
                        chunk_id: score.chunk_id,
                        score: score.score,
                    })
                    .collect::<Vec<_>>();
                cached_item_scores = merge_cached_item_scores(cached_item_scores, persisted_scores);
            }

            if rerank_item_scores_complete(&rerank_chunk_ids, &cached_item_scores) {
                probe.rerank_item_cache_hit = true;
                let sorted_scores = sort_cached_item_scores(&rerank_chunk_ids, &cached_item_scores);
                let reranked = rerank_hits_from_item_scores(&sorted_scores);
                self.cache
                    .set_rerank_batch(
                        generation,
                        &settings.rerank_model,
                        query_hash,
                        rerank_top_n,
                        &candidate_hash,
                        &reranked,
                    )
                    .await;
                (
                    apply_rerank(candidates, &rerank_candidates, &reranked, fetch_limit),
                    true,
                )
            } else {
                let documents = rerank_candidates
                    .iter()
                    .map(|hit| RerankDocument {
                        text: rerank_document_text(hit),
                    })
                    .collect::<Vec<_>>();
                match self
                    .rerank
                    .rerank(&request.query, &documents, rerank_top_n, settings)
                    .await
                {
                    Ok(reranked) => {
                        self.cache
                            .set_rerank_batch(
                                generation,
                                &settings.rerank_model,
                                query_hash,
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
                                query_hash,
                                &item_scores,
                            )
                            .await;
                        let persisted = item_scores
                            .iter()
                            .map(|score| StoredRerankItemScore {
                                rerank_model: settings.rerank_model.clone(),
                                query_hash: query_hash.to_string(),
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
                        (
                            apply_rerank(candidates, &rerank_candidates, &reranked, fetch_limit),
                            true,
                        )
                    }
                    Err(error) => {
                        warn!(error = %error, "rerank failed; falling back to local hybrid ranking");
                        (
                            candidates.into_iter().take(fetch_limit).collect(),
                            false,
                        )
                    }
                }
            }
        };
        probe.rerank_elapsed_ms = rerank_started.elapsed().as_millis() as u64;
        Ok((rerank_outcome, rerank_applied))
    }

    pub async fn search(
        &self,
        user_id: Option<i64>,
        request: SearchRequest,
    ) -> Result<SearchResponse> {
        if request.sort == context69_contracts::SearchSort::Date {
            return self.search_by_date(user_id, request).await;
        }
        let mut probe = SearchProbe::new();
        let offset = self.resolved_offset(&request)?;
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
        let query_hash = SearchCache::query_hash(&request.query);
        let cursor_ctx = cursor_context(&request, request.limit, generation, &settings);
        let vector_multiplier = if request.metadata_filters.is_empty() {
            1
        } else {
            8
        };
        // The vector fetch must cover at least the rerank snapshot
        // (`settings.candidate_limit`) so a single page request still has
        // enough candidates to produce a stable ordering that other pages
        // can reproduce. `max(80, candidate_limit)` covers both small and
        // larger settings.
        let rerank_snapshot_size = settings.candidate_limit.max(1);
        let vector_limit = fetch_limit
            .max(80)
            .max(rerank_snapshot_size)
            .saturating_mul(vector_multiplier)
            .min(MAX_SEARCH_CANDIDATE_WINDOW);
        probe.vector_limit = vector_limit;
        // The response cache stores pages of the ordering that produced them;
        // a cursor pins the request to a specific ordering epoch, so cursor
        // requests always run the pipeline and validate their ordering instead
        // of trusting a cached payload.
        if let Some(response) = self
            .cache
            .get_search_response(generation, &request_hash, &settings_hash)
            .await
            .filter(|_| request.cursor.is_none())
        {
            probe.response_cache_hit = true;
            probe.fetch_limit = fetch_limit;
            probe.offset = offset;
            probe.limit = request.limit;
            probe.has_more = response.pagination.has_more;
            probe.result_count = response.items.len();
            probe.log();
            return Ok(response);
        }

        let collected = self
            .collect_candidates(
                &request,
                &scope,
                &settings,
                &query_hash,
                vector_limit,
                &mut probe,
            )
            .await?;
        let window = PageWindow {
            offset,
            limit: request.limit,
            requested_limit,
            fetch_limit,
            can_probe_window,
            upstream_capped: collected.upstream_capped,
        };
        let ctx = RerankContext {
            settings: &settings,
            request: &request,
            generation,
            query_hash: &query_hash,
            fetch_limit,
        };
        let (results, rerank_applied) = if collected.hybrid {
            self.rerank_stage(collected.candidates, ctx, &mut probe)
                .await?
        } else {
            // Vector mode has no rerank stage: the local ordering is final.
            (collected.candidates, false)
        };

        // A cursor is only valid inside the ordering epoch that issued it;
        // reject cross-ordering use with a clean error instead of silently
        // serving a differently ordered window.
        ensure_cursor_ordering(request.cursor.as_deref(), rerank_applied)?;
        // The cursor also binds the request context (query/filter/limit/
        // generation/settings). Reject mismatched reuse so a cursor for one
        // filter set cannot slice a different candidate window.
        validate_cursor_context(request.cursor.as_deref(), &cursor_ctx)?;
        // Pagination geometry is offset * limit; a misaligned offset would
        // silently produce overlapping windows, so reject it up front.
        validate_cursor_alignment(request.cursor.as_deref())?;

        let (pagination, items) = finalize_window(results, &window, &cursor_ctx, rerank_applied)?;
        let response = SearchResponse {
            query: request.query,
            items,
            pagination,
        };
        if request.cursor.is_none() {
            self.cache
                .set_search_response(generation, &request_hash, &settings_hash, &response)
                .await;
        }
        probe.fetch_limit = fetch_limit;
        probe.offset = offset;
        probe.limit = request.limit;
        probe.has_more = response.pagination.has_more;
        probe.result_count = response.items.len();
        probe.log();
        Ok(response)
    }

    /// Run the date-mode (`sort=date`) pipeline for a POST search. The
    /// pipeline walks Qdrant in a monotonic keyset over `published_ts
    /// DESC`, draining each boundary timestamp in full via the
    /// Qdrant-side `next_page_offset` before advancing to an older
    /// timestamp, and emits an opaque `c4:` cursor for the next page
    /// when more results are visible. No rerank, no response cache, no
    /// hybrid keyword path.
    pub async fn search_by_date(
        &self,
        user_id: Option<i64>,
        request: SearchRequest,
    ) -> Result<SearchResponse> {
        run_date_search(
            &*self.repository,
            &*self.scope_resolver,
            &*self.embedding,
            &*self.index,
            user_id,
            request,
        )
        .await
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

/// Validate an opaque cursor from outside the crate (e.g. the SSE handler
/// wants to reject malformed/oversized payloads before opening the 200
/// stream). Returns `Ok(())` for a missing cursor; the input `raw` may be
/// `None` only when the caller already knows the request is cursor-less.
pub fn decode_search_cursor_for_validation(raw: &str) -> Result<()> {
    let _ = decode_cursor(raw)?;
    Ok(())
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

/// Canonical user-facing match reason derived from the channels that produced a
/// hit plus the title-hit flag already computed by keyword search.
///
/// Tokens: `semantic` (vector/embedding channel), `title` (keyword title exact or
/// phrase match), `keyword` (keyword content match). Multiple reasons are joined
/// with `+`; a hit that matched nothing meaningful falls back to `keyword`.
fn derive_match_reason(vector_score: Option<f32>, keyword_mark: Option<&str>) -> String {
    let mut reasons = Vec::new();

    if vector_score.is_some() {
        reasons.push("semantic");
    }

    match keyword_mark {
        Some("title_exact") | Some("title_phrase") => reasons.push("title"),
        Some("chunk_phrase") | Some("all_terms") => reasons.push("keyword"),
        _ => {}
    }

    if reasons.is_empty() {
        reasons.push("keyword");
    }

    reasons.join("+")
}

#[cfg(test)]
mod tests {
    use context69_contracts::Pagination;

    use super::{
        MAX_SEARCH_CANDIDATE_WINDOW, derive_match_reason, probe_fetch_limit, resolve_search_window,
    };

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
    fn derive_match_reason_encodes_channel_and_title_flags() {
        assert_eq!(derive_match_reason(Some(0.5), None), "semantic");
        assert_eq!(derive_match_reason(None, Some("title_exact")), "title");
        assert_eq!(derive_match_reason(None, Some("title_phrase")), "title");
        assert_eq!(derive_match_reason(None, Some("chunk_phrase")), "keyword");
        assert_eq!(derive_match_reason(None, Some("all_terms")), "keyword");
        assert_eq!(
            derive_match_reason(Some(0.5), Some("title_exact")),
            "semantic+title"
        );
        assert_eq!(
            derive_match_reason(Some(0.5), Some("chunk_phrase")),
            "semantic+keyword"
        );
        assert_eq!(derive_match_reason(None, None), "keyword");
        assert_eq!(derive_match_reason(Some(0.5), Some("unknown")), "semantic");
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
