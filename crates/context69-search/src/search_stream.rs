//! Search stream (SSE) pipeline: stage 1 emits the local-ordering page of the
//! requested window immediately, stage 2 reranks and emits the final ordering
//! of the same window, then `done`. Both stages share one candidate fetch per
//! connection (`SearchService::collect_candidates`).
//!
//! The pipeline never consults the response cache: a cached page only carries
//! the final ordering and cannot reconstruct the stage-1 `local` ordering.
//! Rerank cache hits (batch/item score caches) make stage 2 follow fast while
//! still emitting both events.
//!
//! Client disconnects are propagated through an `AbortSignal`: the SSE
//! handler owns an `AbortOnDrop` whose drop fires when the response body is
//! dropped (client disconnect or unmount), and the pipeline selects on the
//! signal next to its long-running awaits so an in-flight embed/Qdrant/
//! keyword/rerank call returns immediately instead of running to
//! completion.

use anyhow::Result;
use context69_contracts::search::{SearchStreamDone, SearchStreamEvent, SearchStreamPage};
use context69_contracts::{SearchHit, SearchRequest};
use tokio::sync::mpsc;
use tracing::info;

use crate::{AbortSignal, SearchCache, SearchService, SearchSettings};

use super::probe::SearchProbe;
use super::search_cursor::{
    CursorContext, PageWindow, cursor_context, decode_cursor, ensure_cursor_ordering,
    finalize_window, validate_cursor_alignment, validate_cursor_context,
};
use super::search_date::run_date_search;
use super::{MAX_SEARCH_CANDIDATE_WINDOW, RerankContext, probe_fetch_limit};

impl SearchService {
    /// Run the staged search pipeline for one SSE connection, streaming
    /// `local` → (`reranked`) → `done` frames into `tx`. All client-visible
    /// failures are delivered as an in-band `error` frame so the handler can
    /// keep the 200/event-stream response shape. `abort` resolves when the
    /// SSE body is dropped (client disconnect); the pipeline selects on it
    /// next to its long-running awaits so the in-flight work stops.
    pub async fn stream_search(
        &self,
        user_id: Option<i64>,
        request: SearchRequest,
        tx: mpsc::Sender<SearchStreamEvent>,
        abort: AbortSignal,
    ) -> Result<()> {
        if let Err(error) = self.stream_search_inner(user_id, request, &tx, abort).await {
            // Best effort: if the client already disconnected this send fails
            // and there is nothing left to deliver.
            let _ = tx
                .send(SearchStreamEvent::Error {
                    message: error.to_string(),
                })
                .await;
        }
        Ok(())
    }

    async fn stream_search_inner(
        &self,
        user_id: Option<i64>,
        request: SearchRequest,
        tx: &mpsc::Sender<SearchStreamEvent>,
        mut abort: AbortSignal,
    ) -> Result<()> {
        if request.sort == context69_contracts::SearchSort::Date {
            return self.stream_search_by_date(user_id, request, tx, abort).await;
        }
        let mut probe = SearchProbe::new();
        // F2: validate the cursor's geometry, alignment, and context BEFORE
        // any work begins. An invalid cursor must not produce a `local` frame
        // (which would render the wrong window briefly before the error
        // frame). We send the error as the only frame in that case.
        if let Some(raw_cursor) = request.cursor.as_deref() {
            decode_cursor(raw_cursor)?;
            validate_cursor_alignment(Some(raw_cursor))?;
        }
        let offset = self.resolved_offset(&request)?;
        let requested_limit = offset
            .checked_add(request.limit)
            .ok_or_else(|| anyhow::anyhow!("search result limit is too large"))?;
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
        let query_hash = SearchCache::query_hash(&request.query);
        let cursor_ctx = cursor_context(&request, request.limit, generation, &settings);
        // F2: the cursor's request context must match the active request
        // before we emit any `local` frame. A reranked cursor against a
        // vector (local-only) run, or a cursor with a mismatched query,
        // surfaces as the first (and only) frame.
        validate_cursor_context(request.cursor.as_deref(), &cursor_ctx)?;
        let vector_multiplier = if request.metadata_filters.is_empty() {
            1
        } else {
            8
        };
        let rerank_snapshot_size = settings.candidate_limit.max(1);
        let vector_limit = fetch_limit
            .max(80)
            .max(rerank_snapshot_size)
            .saturating_mul(vector_multiplier)
            .min(MAX_SEARCH_CANDIDATE_WINDOW);
        probe.vector_limit = vector_limit;

        // F1: collect candidates under the abort signal so a client
        // disconnect stops the embed/vector/keyword fetch.
        let collected = tokio::select! {
            biased;
            _ = abort.wait() => return Ok(()),
            result = self.collect_candidates(
                &request,
                &scope,
                &settings,
                &query_hash,
                vector_limit,
                &mut probe,
            ) => result?,
        };
        let window = PageWindow {
            offset,
            limit: request.limit,
            requested_limit,
            fetch_limit,
            can_probe_window,
            upstream_capped: collected.upstream_capped,
        };

        // F2: a cursor pins the request to one ordering epoch. We must
        // resolve the epoch before emitting any `local` frame so a cursor
        // that no longer matches the active ordering surfaces as the only
        // frame the client ever sees.
        if let Some(raw_cursor) = request.cursor.as_deref() {
            // Vector mode is local-only (`rerank_applied = false`) so the
            // effective epoch is known up front. Reject a reranked cursor
            // before any work could leak a `local` frame.
            if !collected.hybrid {
                if let Err(error) = ensure_cursor_ordering(Some(raw_cursor), false) {
                    send_error(tx, &error.to_string()).await;
                    return Ok(());
                }
            } else {
                // Hybrid mode: the actual `rerank_applied` is decided by
                // the rerank stage, so run it under the abort signal first,
                // then validate the cursor against the real outcome. The
                // candidates are reused — we never fetch twice. We snapshot
                // the local-ordering slice before rerank so stage 1 still
                // carries the local ordering even when rerank succeeds.
                let local_ordering: Vec<SearchHit> = collected
                    .candidates
                    .iter()
                    .take(fetch_limit)
                    .cloned()
                    .collect();
                let ctx = RerankContext {
                    settings: &settings,
                    request: &request,
                    generation,
                    query_hash: &query_hash,
                    fetch_limit,
                };
                let (rerank_results, rerank_applied) = tokio::select! {
                    biased;
                    _ = abort.wait() => return Ok(()),
                    result = self.rerank_stage(collected.candidates, ctx, &mut probe) => result?,
                };
                if let Err(error) = ensure_cursor_ordering(Some(raw_cursor), rerank_applied) {
                    send_error(tx, &error.to_string()).await;
                    return Ok(());
                }
                // Cursor validates: emit the local page for the same window,
                // then (only when rerank actually reordered) the reranked
                // page, then `done`.
                let local_page =
                    self.finalize_page(local_ordering, &window, &cursor_ctx, false)?;
                self.fill_probe_window(&mut probe, fetch_limit, offset, request.limit, &local_page);
                if !send_frame(tx, SearchStreamEvent::Local(local_page)).await {
                    return Ok(());
                }
                self.log_local_stage(&probe, "hybrid");
                if rerank_applied {
                    let reranked_page =
                        self.finalize_page(rerank_results, &window, &cursor_ctx, true)?;
                    self.fill_probe_window(
                        &mut probe,
                        fetch_limit,
                        offset,
                        request.limit,
                        &reranked_page,
                    );
                    if !send_frame(tx, SearchStreamEvent::Reranked(reranked_page)).await {
                        return Ok(());
                    }
                    info!(
                        rerank_elapsed_ms = probe.rerank_elapsed_ms,
                        rerank_top_n = probe.rerank_top_n,
                        rerank_candidate_len = probe.rerank_candidate_len,
                        rerank_batch_cache_hit = probe.rerank_batch_cache_hit,
                        rerank_item_cache_hit = probe.rerank_item_cache_hit,
                        has_more = ?probe.has_more,
                        result_count = probe.result_count,
                        offset = probe.offset,
                        limit = probe.limit,
                        "search stream stage2 (reranked ordering) page emitted"
                    );
                } else {
                    info!(
                        rerank_elapsed_ms = probe.rerank_elapsed_ms,
                        reason = "rerank disabled, empty candidate set, or upstream rerank failed",
                        has_more = ?probe.has_more,
                        result_count = probe.result_count,
                        offset = probe.offset,
                        limit = probe.limit,
                        "search stream stage2 kept local ordering"
                    );
                }
                let _ = send_frame(
                    tx,
                    SearchStreamEvent::Done(SearchStreamDone { rerank_applied }),
                )
                .await;
                return Ok(());
            }
        }

        // ---- Stage 1: emit the local-ordering page immediately ---------------
        if !collected.hybrid {
            // Vector mode has no rerank stage: one ordering, one page.
            let page = self.finalize_page(collected.candidates, &window, &cursor_ctx, false)?;
            self.fill_probe_window(&mut probe, fetch_limit, offset, request.limit, &page);
            if !send_frame(tx, SearchStreamEvent::Local(page)).await {
                return Ok(());
            }
            self.log_local_stage(&probe, "vector");
            let _ = send_frame(
                tx,
                SearchStreamEvent::Done(SearchStreamDone {
                    rerank_applied: false,
                }),
            )
            .await;
            return Ok(());
        }

        // Hybrid stage 1 (cursor-less request): local ordering is derived
        // from the same candidates that stage 2 will rerank, so the fetch
        // happens exactly once.
        let local_page = self.finalize_page(
            collected
                .candidates
                .iter()
                .take(fetch_limit)
                .cloned()
                .collect::<Vec<_>>(),
            &window,
            &cursor_ctx,
            false,
        )?;
        self.fill_probe_window(&mut probe, fetch_limit, offset, request.limit, &local_page);
        if !send_frame(tx, SearchStreamEvent::Local(local_page)).await {
            return Ok(());
        }
        self.log_local_stage(&probe, "hybrid");

        // ---- Stage 2: rerank stable snapshot and emit the final ordering -----
        let ctx = RerankContext {
            settings: &settings,
            request: &request,
            generation,
            query_hash: &query_hash,
            fetch_limit,
        };
        // F1: rerank under the abort signal so a client disconnect stops the
        // upstream rerank call instead of running it to completion.
        let (final_results, rerank_applied) = tokio::select! {
            biased;
            _ = abort.wait() => return Ok(()),
            result = self.rerank_stage(collected.candidates, ctx, &mut probe) => result?,
        };

        // A cursor is only valid inside the ordering epoch that issued it;
        // never silently continue with a differently ordered window.
        // F2: rerank may have failed and changed the effective epoch; if the
        // cursor was issued for the reranked ordering we must not silently
        // continue with the local ordering. The error frame is the only
        // thing the client sees in that case (no `local` was pre-validated
        // for a different epoch).
        if let Err(error) = ensure_cursor_ordering(request.cursor.as_deref(), rerank_applied) {
            send_error(tx, &error.to_string()).await;
            return Ok(());
        }

        if rerank_applied {
            let reranked_page = self.finalize_page(final_results, &window, &cursor_ctx, true)?;
            self.fill_probe_window(
                &mut probe,
                fetch_limit,
                offset,
                request.limit,
                &reranked_page,
            );
            if !send_frame(tx, SearchStreamEvent::Reranked(reranked_page)).await {
                return Ok(());
            }
            info!(
                rerank_elapsed_ms = probe.rerank_elapsed_ms,
                rerank_top_n = probe.rerank_top_n,
                rerank_candidate_len = probe.rerank_candidate_len,
                rerank_batch_cache_hit = probe.rerank_batch_cache_hit,
                rerank_item_cache_hit = probe.rerank_item_cache_hit,
                has_more = ?probe.has_more,
                result_count = probe.result_count,
                offset = probe.offset,
                limit = probe.limit,
                "search stream stage2 (reranked ordering) page emitted"
            );
        } else {
            info!(
                rerank_elapsed_ms = probe.rerank_elapsed_ms,
                reason = "rerank disabled, empty candidate set, or upstream rerank failed",
                has_more = ?probe.has_more,
                result_count = probe.result_count,
                offset = probe.offset,
                limit = probe.limit,
                "search stream stage2 kept local ordering"
            );
        }
        let _ = send_frame(
            tx,
            SearchStreamEvent::Done(SearchStreamDone { rerank_applied }),
        )
        .await;
        Ok(())
    }

    /// Turn an ordered result list into a stream page for `window`.
    fn finalize_page(
        &self,
        results: Vec<SearchHit>,
        window: &PageWindow,
        context: &CursorContext,
        rerank_applied: bool,
    ) -> Result<SearchStreamPage> {
        let (pagination, items) = finalize_window(results, window, context, rerank_applied)?;
        Ok(SearchStreamPage { items, pagination })
    }

    fn fill_probe_window(
        &self,
        probe: &mut SearchProbe,
        fetch_limit: usize,
        offset: usize,
        limit: usize,
        page: &SearchStreamPage,
    ) {
        probe.fetch_limit = fetch_limit;
        probe.offset = offset;
        probe.limit = limit;
        probe.has_more = page.pagination.has_more;
        probe.result_count = page.items.len();
    }

    /// Run the date-mode streaming pipeline: one `date` (or `local`) frame
    /// per assembled batch plus a terminal `done` event. No `reranked` frame
    /// (date mode never re-orders the results). Abort and cursor context
    /// validation follow the same rules as the relevance stream: nothing is
    /// emitted before the cursor and the abort signal stop the work.
    async fn stream_search_by_date(
        &self,
        user_id: Option<i64>,
        request: SearchRequest,
        tx: &mpsc::Sender<SearchStreamEvent>,
        mut abort: AbortSignal,
    ) -> Result<()> {
        // F2: validate the cursor's geometry, alignment, and context BEFORE
        // any work begins. An invalid cursor must not produce a `date`
        // frame (which would render the wrong window briefly before the
        // error frame). We send the error as the only frame in that case.
        if let Some(raw_cursor) = request.cursor.as_deref() {
            // The relevance decoder already rejects unknown prefixes; for a
            // date stream we additionally require a `c4:` cursor.
            super::search_cursor::validate_date_cursor(Some(raw_cursor))?;
        }
        let outcome = tokio::select! {
            biased;
            _ = abort.wait() => return Ok(()),
            result = run_date_search(
                &*self.repository,
                &*self.scope_resolver,
                &*self.embedding,
                &*self.index,
                user_id,
                request,
            ) => result,
        };
        let response = match outcome {
            Ok(response) => response,
            Err(error) => {
                let _ = tx
                    .send(SearchStreamEvent::Error {
                        message: error.to_string(),
                    })
                    .await;
                return Ok(());
            }
        };
        let page = SearchStreamPage {
            items: response.items,
            pagination: response.pagination,
        };
        if !send_frame(tx, SearchStreamEvent::Local(page)).await {
            return Ok(());
        }
        // Date mode has no rerank stage; we still emit a `done` frame so
        // clients with a streaming renderer can stop showing a spinner.
        let _ = send_frame(
            tx,
            SearchStreamEvent::Done(SearchStreamDone {
                rerank_applied: false,
            }),
        )
        .await;
        Ok(())
    }

    fn log_local_stage(&self, probe: &SearchProbe, mode: &str) {
        info!(
            mode,
            embed_cache_hit = probe.embed_cache_hit,
            embed_elapsed_ms = probe.embed_elapsed_ms,
            vector_elapsed_ms = probe.vector_elapsed_ms,
            vector_candidate_count = probe.vector_candidate_count,
            vector_limit = probe.vector_limit,
            hydrate_elapsed_ms = probe.hydrate_elapsed_ms,
            hydrate_candidate_count = probe.hydrate_candidate_count,
            keyword_elapsed_ms = probe.keyword_elapsed_ms,
            keyword_candidate_count = probe.keyword_candidate_count,
            keyword_limit = ?probe.keyword_limit,
            merge_elapsed_ms = probe.merge_elapsed_ms,
            merged_candidate_count = probe.merged_candidate_count,
            has_more = ?probe.has_more,
            result_count = probe.result_count,
            offset = probe.offset,
            limit = probe.limit,
            "search stream stage1 (local ordering) page emitted"
        );
    }
}

/// Deliver an `error` frame without aborting the caller when the client is
/// gone; returns whether delivery succeeded.
async fn send_error(tx: &mpsc::Sender<SearchStreamEvent>, message: &str) -> bool {
    send_frame(
        tx,
        SearchStreamEvent::Error {
            message: message.to_string(),
        },
    )
    .await
}

/// Deliver one frame; `false` means the client disconnected and the remaining
/// pipeline must stop.
async fn send_frame(tx: &mpsc::Sender<SearchStreamEvent>, event: SearchStreamEvent) -> bool {
    tx.send(event).await.is_ok()
}
