//! Date-mode (`sort=date`) search pipeline.
//!
//! Walks Qdrant in a monotonic keyset over `published_ts DESC`, latest first,
//! until the requested page size is filled, the index is exhausted, or the
//! request-level `MAX_DATE_WINDOWS` cap is reached. No rerank, no response
//! cache, no hybrid keyword path.
//!
//! The query text is honoured: a non-blank `query` is matched against the
//! hydrated `title + chunk_text` with the same all-terms substring rule used
//! by the keyword pipeline in `src/db/documents.rs::keyword_search` (the
//! `keyword_terms` split + every-term substring). When the normalized query
//! is non-blank but the split produces zero terms (e.g. only punctuation /
//! whitespace), the matcher falls back to a literal phrase substring over the
//! lowercased `title + ' ' + chunk_text`, mirroring the SQL
//! `lower(title) LIKE phrase OR lower(chunk) LIKE phrase` shape used by the
//! keyword path. A blank query in date mode is a 400 validation error so a
//! `sort=date` request never silently turns into a "latest N" browse.
//!
//! The walk is a two-tier keyset driven by the Qdrant `scroll` return order:
//!   * **Cross-timestamp fetch**: ordered by `published_ts DESC` with a
//!     fixed inclusive `before` bound. Returns a batch of records spanning
//!     one or more timestamps. The pipeline groups the batch by timestamp
//!     and identifies the newest timestamp; the in-flight `boundary_ts` is
//!     set to that value and the next iteration drains it.
//!   * **Per-timestamp drain**: with `boundary_ts` set, every fetch is
//!     filtered to `published_ts == boundary_ts` and paged via the
//!     Qdrant-side `next_page_offset` (a `PointId` UUID) so the entire
//!     boundary population is collected before advancing to an older
//!     timestamp. The Qdrant `scroll` return order is the authoritative
//!     ordering inside a same-second boundary — the pipeline does NOT
//!     re-sort in memory. The Qdrant-side `next_offset` is threaded back
//!     into the next call as the `offset` argument so a request can resume
//!     the boundary across requests without losing records.
//!
//! Determinism within one ordering epoch: the Qdrant `scroll` return order
//! over `published_ts DESC` is deterministic for a given filter set + offset
//! + generation. The cursor pins the request context (query / filter /
//! settings / generation) AND the `next_page_offset`, so a replayed request
//! resumes exactly after the last emitted record. The `seen` set still
//! guarantees no record is emitted twice within one request.
//!
//! The snapshot ceiling (`upper`) is captured under the caller's access
//! scope on the first request and is replayed unchanged in the cursor so
//! freshly indexed records do not silently widen the result set. Snapshot
//! errors propagate as `Err(_)`; `Ok(None)` means the index reported an
//! actually empty visible window and is the only signal the pipeline
//! treats as "no upper bound".
//!
//! The request-level `MAX_DATE_WINDOWS` cap is a per-request budget: it is
//! NOT accumulated in the cursor, and a replayed request always starts
//! with a fresh budget. Any loop exit that may still hold undrained state
//! (per-timestamp cap hit, full page, open boundary, present offset, or
//! request-level cap hit) emits a resumable cursor + `has_more = true` so
//! the next request can continue the walk. `has_more = false` + no cursor
//! is only emitted when a fetch returns fewer than the requested limit and
//! no boundary / offset state remains.
//!
//! Termination: every cross-timestamp advance strictly decreases `before`
//! (`ts - 1`); every per-timestamp drain strictly advances `offset` until
//! Qdrant reports exhaustion; the per-request window cap bounds the work
//! without ever producing a `has_more = false` while records remain.

use std::collections::HashSet;

use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use context69_contracts::{SearchHit, SearchRequest, SearchResponse};
use tracing::info;

use super::search_cursor::{
    CursorContext, DateCursor, MAX_DATE_WINDOWS, encode_date_cursor, validate_date_cursor,
};
use crate::{
    DateBound, DateWindowPage, DateWindowQuery, SearchCache, SearchDatePointHit,
    SearchEmbeddingProvider, SearchIndex, SearchRepository, SearchScopeResolver, SearchSettings,
};

/// Hard cap on the index-side `limit` for one date-mode window fetch.
///
/// The cross-timestamp discovery fetch is generous so the pipeline can
/// collect a full boundary population (multiple timestamps) without
/// artificially fragmenting the walk. Subsequent per-timestamp drains
/// use the user-requested `limit` (not this cap) so the per-call
/// pagination is bounded by the page size — that way a same-second
/// boundary whose population exceeds `limit` is drained across
/// multiple `scroll` calls with `next_page_offset` threaded through
/// the cursor.
///
/// Cross-request resume uses the same key: the cursor carries the
/// Qdrant-side `offset` (PointId UUID) plus the `boundary_ts` and
/// `upper` so a replayed request resumes the boundary exactly where
/// the previous one stopped. A popped boundary (one whose population
/// fits inside a single fetch) reports `next_offset = None` and the
/// pipeline advances below the boundary on the next iteration.
pub(crate) const DATE_BOUNDARY_FETCH_LIMIT: usize = 4_096;

#[derive(Debug, Clone)]
struct DateRequestSnapshot {
    settings: SearchSettings,
    generation: i64,
    query_hash: String,
    /// Snapshot `published_ts` ceiling captured under the caller's scope.
    /// `None` means the index reported an empty visible window.
    upper_ts: Option<i64>,
}

impl DateRequestSnapshot {
    async fn capture(
        repository: &dyn SearchRepository,
        user_id: Option<i64>,
        request: &SearchRequest,
    ) -> Result<Self> {
        let settings = repository
            .get_search_settings()
            .await?
            .unwrap_or_else(SearchSettings::default);
        let generation = repository.get_search_generation().await?;
        let query_hash = SearchCache::query_hash(&request.query);
        // Snapshot failure must propagate as a 500; the date pipeline
        // distinguishes "no visible upper bound" (Ok(None)) from
        // "snapshot fetch failed" (Err(_)) so a transient Qdrant outage
        // can never silently widen a replayed upper bound.
        let upper_ts = repository.date_request_upper_bound(user_id, request).await?;
        Ok(Self {
            settings,
            generation,
            query_hash,
            upper_ts,
        })
    }
}

#[derive(Debug, Default)]
struct DateAccumulator {
    items: Vec<SearchHit>,
    seen: HashSet<uuid::Uuid>,
    /// True when the index returned at least one matching record during
    /// the run; helps decide between `has_more = false` and
    /// `has_more = true` when the page is full.
    pushed_in_run: bool,
    /// True when a same-second drain is in flight at the end of the run.
    /// The next cursor must resume the boundary with the same
    /// `boundary_ts` and the last emitted Qdrant offset so no record is
    /// skipped or duplicated across the page boundary.
    boundary_open: bool,
    /// The most recent `next_offset` the Qdrant side returned for the
    /// boundary, even if no record was pushed. The cursor carries this
    /// so a replay can re-issue the same `scroll` call and pick up the
    /// remainder of the boundary.
    boundary_next_offset: Option<uuid::Uuid>,
}

impl DateAccumulator {
    fn push(&mut self, hit: SearchHit) -> bool {
        if self.seen.insert(hit.chunk_id) {
            self.items.push(hit);
            true
        } else {
            false
        }
    }
}

/// Match a hydrated hit against the lowercased `keyword_terms` derived from
/// the request query. When the normalized query is non-blank but the split
/// produces zero terms (e.g. the query is only punctuation / whitespace), the
/// matcher falls back to a literal phrase substring over the lowercased
/// `title + ' ' + chunk_text` — the SQL equivalent of
/// `lower(title) LIKE phrase OR lower(chunk) LIKE phrase` — so the date
/// pipeline matches the keyword path's behaviour byte-for-byte. With one or
/// more terms, the all-terms substring rule is applied.
fn matches_keyword_terms(hit: &SearchHit, terms: &[String], phrase: &str) -> bool {
    if terms.is_empty() {
        if phrase.is_empty() {
            // The request guard rejects a blank query before this point;
            // a non-blank query with only punctuation / whitespace is the
            // only path that lands here with a non-empty phrase. The
            // literal phrase substring acts as the cardinality guard: a
            // match requires the exact phrase to appear in either the
            // title or the chunk text, so a query of `"---"` cannot
            // silently match every record.
            return false;
        }
        let mut haystack = String::with_capacity(hit.title.len() + hit.chunk_text.len() + 1);
        haystack.push_str(&hit.title.to_lowercase());
        haystack.push(' ');
        haystack.push_str(&hit.chunk_text.to_lowercase());
        return haystack.contains(phrase);
    }
    let mut haystack = String::with_capacity(hit.title.len() + hit.chunk_text.len() + 1);
    haystack.push_str(&hit.title.to_lowercase());
    haystack.push(' ');
    haystack.push_str(&hit.chunk_text.to_lowercase());
    terms
        .iter()
        .all(|term| !term.is_empty() && haystack.contains(term))
}

/// Build the `keyword_terms` vector and the lowercased literal phrase for
/// the current query. The split rule is the same as
/// `src/db/rows.rs::keyword_terms`: split on whitespace or ASCII
/// punctuation, trim, drop empties. The lowercased tokens are what
/// `matches_keyword_terms` expects; the lowercased phrase is the literal
/// fallback when the split produces zero terms (e.g. the query is only
/// punctuation / dashes).
fn keyword_query_parts(query: &str) -> (Vec<String>, String) {
    let phrase = query.trim().to_lowercase();
    let terms: Vec<String> = phrase
        .split(|ch: char| ch.is_whitespace() || ch.is_ascii_punctuation())
        .map(str::trim)
        .filter(|term| !term.is_empty())
        .map(ToOwned::to_owned)
        .collect();
    (terms, phrase)
}

/// Validate a `SearchRequest` for the date-mode pipeline. A blank query
/// in date mode is a 400: the contract is "matching query, latest-first,
/// no rerank" and silently turning the request into a browse would
/// violate it. The HTTP layer also pre-validates this; the service-side
/// check is the authoritative guard.
fn validate_date_query(request: &SearchRequest) -> Result<()> {
    if request.query.trim().is_empty() {
        return Err(anyhow!(
            "query text is required for sort=date; the date pipeline matches the query against hydrated hits (title + chunk_text) and never browses the index"
        ));
    }
    Ok(())
}

/// Run the date-mode pipeline for a POST search or an SSE stream.
///
/// Cancellation is owned by the SSE layer: the `stream_search_by_date`
/// handler wraps the entire `run_date_search` future in a
/// `tokio::select!` against the `AbortSignal`, so a client disconnect
/// drops the in-flight Qdrant / hydration / filter work instead of
/// running it to completion. The POST path does not need a separate
/// abort signal.
pub async fn run_date_search(
    repository: &dyn SearchRepository,
    scope_resolver: &dyn SearchScopeResolver,
    embedding: &dyn SearchEmbeddingProvider,
    index: &dyn SearchIndex,
    user_id: Option<i64>,
    request: SearchRequest,
) -> Result<SearchResponse> {
    let limit = request.limit;
    if limit == 0 || limit > 100 {
        return Err(anyhow!("page_size must be between 1 and 100"));
    }
    // Date mode is "query-matching, latest-first, no rerank" — a blank
    // query would silently become a browse, so the request is rejected
    // before any expensive work.
    validate_date_query(&request)?;
    if request.cursor.is_none() && request.page > 1 {
        // Date mode is forward-only keyset pagination: the legacy
        // `(page - 1) * limit` mapping has no meaning here and silently
        // serving page 1 would be a worse failure than a clean 400.
        return Err(anyhow!(
            "page > 1 is not supported for sort=date; pass the cursor returned in `next_cursor` to fetch the next page"
        ));
    }
    let snapshot = DateRequestSnapshot::capture(repository, user_id, &request).await?;
    let context = CursorContext {
        query_hash: snapshot.query_hash.clone(),
        filter_hash: SearchCache::filter_hash(&request),
        generation: snapshot.generation,
        settings_hash: SearchCache::settings_hash(&snapshot.settings),
        limit,
    };
    let scope = scope_resolver
        .access_scope(user_id, request.group_path.clone())
        .await?;
    let incoming = match request.cursor.as_deref() {
        Some(raw) => {
            let date = validate_date_cursor(Some(raw))?;
            validate_date_context(&date, &context)?;
            Some(date)
        }
        None => None,
    };
    // The first window's upper edge is the snapshot ceiling. A replayed
    // cursor carries the same `upper` so a freshly indexed record cannot
    // silently widen the replayed range.
    let mut before: Option<i64> = incoming
        .as_ref()
        .and_then(|d| d.before)
        .or(snapshot.upper_ts);
    let mut boundary_ts: Option<i64> = incoming.as_ref().and_then(|d| d.boundary_ts);
    // The Qdrant-side `next_page_offset` carried by the cursor. `None`
    // means either this is a fresh first page or the previous request
    // fully drained the boundary. The pipeline threads the offset
    // through every per-timestamp fetch so the Qdrant return order
    // continues exactly after the last emitted record.
    let mut boundary_offset: Option<uuid::Uuid> = incoming.as_ref().and_then(|d| d.offset);

    let upper = incoming.as_ref().and_then(|d| d.upper).or(snapshot.upper_ts);

    let vector = embed_once(embedding, &request).await?;

    // All-terms substring rule (or literal phrase fallback). The vector
    // above is computed for service-side embedding observability; the
    // date pipeline filters hits by query text, not by similarity, so the
    // rule is applied to the hydrated `title + chunk_text` after the
    // Qdrant window returns.
    let (keyword_terms, keyword_phrase) = keyword_query_parts(&request.query);

    let mut accumulator = DateAccumulator::default();
    // `hit_exhausted` is the only signal that flips `has_more` to
    // `false`. It fires when a cross-timestamp fetch returns no records,
    // meaning the snapshot ceiling plus the wall-clock `upper` together
    // confirm the index is empty. Any other exit (cap hit, full page,
    // open boundary, present offset) keeps `hit_exhausted = false` and
    // emits a resumable cursor + `has_more = true`.
    let mut hit_exhausted = false;
    // `cap_fired` records whether the per-request `MAX_DATE_WINDOWS`
    // cap stopped the walk. When the cap fires mid-walk the pipeline
    // must still surface a resumable cursor (the request is over its
    // budget, not the result set); the next request starts with a
    // fresh budget and resumes the walk.
    let mut cap_fired = false;
    // Per-request window budget. The cap is per-request: the cursor no
    // longer carries a `windows_consumed` counter, so a replayed
    // request always starts with a fresh budget. When the cap fires
    // mid-walk the loop breaks and the pipeline emits a resumable
    // cursor + `has_more = true` so the next request can continue the
    // walk. One window = one walk step (one cross-timestamp fetch OR
    // one per-timestamp fetch); the per-timestamp drain may issue
    // multiple windows for the same boundary.
    let mut windows_consumed: usize = 0;

    while accumulator.items.len() < limit && !hit_exhausted {
        if windows_consumed >= MAX_DATE_WINDOWS {
            cap_fired = true;
            break;
        }
        if let Some(ts) = boundary_ts {
            // Per-timestamp drain. The Qdrant-side `next_page_offset`
            // pages through the population at `ts` until Qdrant reports
            // exhaustion. `boundary_offset` is the resume key returned
            // by the previous window; the first call uses the cursor's
            // own offset (if any) so a replayed drain picks up exactly
            // where the previous run stopped. The Qdrant `scroll` return
            // order is the authoritative ordering — the pipeline does
            // NOT re-sort in memory, so a reissued call with the same
            // offset returns the same records in the same order.
            //
            // The per-call fetch limit is the user-requested `limit` (not
            // `DATE_BOUNDARY_FETCH_LIMIT`) so a same-second boundary
            // whose population exceeds `limit` is drained across multiple
            // `scroll` calls with the Qdrant-side offset threaded back
            // into the cursor. This is the only mechanism that can emit
            // a resumable cursor when the page fills before the boundary
            // is fully drained.
            let remaining = limit.saturating_sub(accumulator.items.len());
            if remaining == 0 {
                break;
            }
            let batch = fetch_boundary(
                index,
                vector.clone(),
                &request,
                &scope,
                ts,
                boundary_offset,
                remaining,
            )
            .await?;
            windows_consumed += 1;
            let pushed = if batch.hits.is_empty() {
                false
            } else {
                drain_boundary(DrainBoundaryInputs {
                    repository,
                    request: &request,
                    scope: &scope,
                    accumulator: &mut accumulator,
                    raw_hits: batch.hits,
                    limit,
                    keyword_terms: &keyword_terms,
                    keyword_phrase: &keyword_phrase,
                })
                .await?
            };
            if !pushed {
                if let Some(offset) = batch.next_offset {
                    // The Qdrant call returned no new records at this
                    // boundary — every record was either already in
                    // `seen`, dropped by `is_meaningful_text`, dropped
                    // by `metadata_filters_match`, or dropped by
                    // `matches_keyword_terms` — but the Qdrant side
                    // still has more records at `ts`. Conflating
                    // "this slice had no match" with "boundary
                    // exhausted" silently strands the remainder of
                    // the boundary: the next cross-ts fetch at
                    // `before = ts - 1` returns empty, `hit_exhausted`
                    // flips to `true`, and the pipeline reports
                    // `has_more = false` even though undrained
                    // records remain. Keep the in-flight boundary
                    // open and continue with the Qdrant-side resume
                    // key. Only advance past the boundary when Qdrant
                    // proves exhaustion (the fetch returned an empty
                    // page AND `next_offset` is `None`).
                    boundary_offset = Some(offset);
                    accumulator.boundary_next_offset = Some(offset);
                    accumulator.boundary_open = true;
                    continue;
                }
                // Boundary exhausted. The Qdrant call returned no new
                // records at this timestamp (every record was either
                // already in `seen` or the index reported an empty
                // page) and reported no further `next_offset`. Advance
                // to the next-older timestamp and reset the in-flight
                // state so the next iteration's cross-timestamp fetch
                // starts at `before = ts - 1` and the boundary is never
                // revisited.
                boundary_ts = None;
                boundary_offset = None;
                accumulator.boundary_open = false;
                accumulator.boundary_next_offset = None;
                before = ts.checked_sub(1);
                continue;
            }
            if accumulator.items.len() >= limit {
                // The page is full. The cursor must carry the resume
                // offset so the next request can pick up the rest of
                // the boundary; otherwise the in-memory `seen` set has
                // already covered the entire population.
                accumulator.boundary_open = true;
                if let Some(offset) = batch.next_offset {
                    accumulator.boundary_next_offset = Some(offset);
                } else {
                    // The boundary at `ts` is fully drained (Qdrant
                    // reported no more records at this timestamp)
                    // but the page is full so the walk must continue
                    // at the next-older timestamp. Drop the in-flight
                    // boundary and advance `before` so the next
                    // request's cross-timestamp fetch starts one
                    // second older. The cursor's `before` carries the
                    // new upper bound so the replayed request resumes
                    // exactly where this one stopped.
                    boundary_ts = None;
                    accumulator.boundary_open = false;
                    accumulator.boundary_next_offset = None;
                    before = ts.checked_sub(1);
                }
                break;
            }
            if let Some(offset) = batch.next_offset {
                // More records at the same `boundary_ts` exist on the
                // Qdrant side; keep draining with the resume key. The
                // next iteration's `fetch_boundary` call will pass
                // `boundary_offset = Some(offset)` and Qdrant will
                // return the next slice in the same ordering.
                boundary_offset = Some(offset);
                accumulator.boundary_next_offset = Some(offset);
                accumulator.boundary_open = true;
                continue;
            }
            // The Qdrant side reported no more records at this
            // timestamp. The boundary is fully drained; advance to the
            // next-older timestamp on the next iteration.
            boundary_ts = None;
            boundary_offset = None;
            accumulator.boundary_open = false;
            accumulator.boundary_next_offset = None;
            before = ts.checked_sub(1);
            continue;
        }
        // Cross-timestamp fetch.
        let page = fetch_window(
            index,
            vector.clone(),
            &request,
            &scope,
            before,
            DATE_BOUNDARY_FETCH_LIMIT,
        )
        .await?;
        windows_consumed += 1;
        if page.hits.is_empty() {
            hit_exhausted = true;
            break;
        }
        // The Qdrant side returns records in `published_ts DESC` order.
        // The newest timestamp in the batch is the next boundary; the
        // rest of the batch is discarded for this run (those records
        // are older and will be revisited only if additional pages are
        // requested).
        let newest_ts = page
            .hits
            .iter()
            .filter_map(|hit| hit.published_ts)
            .max();
        let Some(newest) = newest_ts else {
            // Records without a `published_ts` are out-of-band: skip
            // them by advancing `before` below the floor so they are
            // never revisited. This matches the old contract that
            // `published_ts` is required for date ordering.
            before = Some(0);
            continue;
        };
        let boundary_hits: Vec<SearchDatePointHit> = page
            .hits
            .into_iter()
            .filter(|hit| hit.published_ts == Some(newest))
            .collect();
        if boundary_hits.is_empty() {
            // Defensive: the batch was non-empty but every record was
            // already past `newest`. This cannot happen because we
            // derived `newest` from the batch, but guard anyway.
            before = newest.checked_sub(1);
            continue;
        }
        boundary_ts = Some(newest);
        // The cross-timestamp fetch returns records spanning multiple
        // timestamps; the per-timestamp drain is the only mechanism
        // that pages through the boundary in `limit`-sized slices. The
        // first per-timestamp drain call uses a fresh offset so the
        // Qdrant side returns the head of the boundary in its
        // authoritative scroll order. Any cross-timestamp
        // `next_offset` is discarded because it may include records at
        // smaller timestamps (the cross-timestamp scroll is not a
        // pure boundary walk).
        boundary_offset = None;
        accumulator.boundary_open = true;
        accumulator.boundary_next_offset = None;
        let _pushed = drain_boundary(DrainBoundaryInputs {
            repository,
            request: &request,
            scope: &scope,
            accumulator: &mut accumulator,
            // The cross-timestamp batch is discarded for the drain
            // step: the per-timestamp fetch is issued from the next
            // iteration so the Qdrant-side `next_page_offset` is the
            // authoritative boundary resume key.
            raw_hits: Vec::new(),
            limit,
            keyword_terms: &keyword_terms,
            keyword_phrase: &keyword_phrase,
        })
        .await?;
        // The drain step above is a no-op (raw_hits is empty). The
        // next loop iteration runs the per-timestamp fetch with a
        // fresh offset and a `limit`-sized slice. Continue without
        // checking the page fill so the per-timestamp fetch has a
        // chance to push the first chunk.
        continue;
    }

    // Decide `has_more`:
    //   * `has_more = true` whenever the page is full AND the index is
    //     not proven exhausted (`hit_exhausted == false`); the next
    //     request will keep walking.
    //   * `has_more = true` whenever a per-timestamp drain is in
    //     flight (`boundary_open`) and the Qdrant side has more
    //     records at the boundary (`boundary_next_offset.is_some()`).
    //   * `has_more = true` whenever a per-timestamp drain is in
    //     flight and the page did not fill — even if Qdrant reported
    //     no further offset, the `seen` set only covered records at
    //     this boundary; older records may still remain.
    //   * `has_more = true` whenever the per-request window cap fired
    //     mid-walk; the next request must resume the walk.
    //   * Otherwise `has_more = false` (page short, or index
    //     exhausted).
    let page_full = accumulator.items.len() == limit;
    let boundary_more = accumulator.boundary_open;
    let has_more = (page_full && !hit_exhausted) || boundary_more || cap_fired;

    let next_cursor = if has_more {
        let next = DateCursor {
            limit,
            query_hash: context.query_hash.clone(),
            filter_hash: context.filter_hash.clone(),
            generation: context.generation,
            settings_hash: context.settings_hash.clone(),
            before,
            boundary_ts,
            // Thread the Qdrant-side resume key. When the boundary is
            // open and Qdrant reported a `next_offset`, the cursor
            // carries it so the next request re-fetches the boundary
            // with that offset and continues the walk in the same
            // ordering. When the boundary is open but Qdrant reported
            // no further offset, the cursor carries `None` so the next
            // request restarts the boundary and the `seen` set skips
            // the records the previous request emitted.
            offset: if accumulator.boundary_open {
                accumulator.boundary_next_offset
            } else {
                None
            },
            upper,
        };
        Some(encode_date_cursor(&next))
    } else {
        // The cap fired but there is no in-flight boundary (the
        // per-ts fetch already advanced `before` past the boundary).
        // The cursor's `before` is strictly less than the snapshot
        // `upper` so the next request must resume the walk. Surface a
        // cursor with no offset so the cursor decodes cleanly and the
        // `before` drives the next cross-timestamp fetch.
        if cap_fired {
            let next = DateCursor {
                limit,
                query_hash: context.query_hash.clone(),
                filter_hash: context.filter_hash.clone(),
                generation: context.generation,
                settings_hash: context.settings_hash.clone(),
                before,
                boundary_ts: None,
                offset: None,
                upper,
            };
            Some(encode_date_cursor(&next))
        } else {
            None
        }
    };
    let mut pagination = context69_contracts::search::SearchPagination::try_new_search_window(
        1,
        u32::try_from(limit)?,
        accumulator.items.len() as u64,
        if has_more { Some(true) } else { Some(false) },
    )?;
    pagination.next_cursor = next_cursor;
    pagination.prev_cursor = None;
    pagination.has_more = if has_more { Some(true) } else { Some(false) };
    pagination.total_is_exact = Some(false);
    info!(
        collected = accumulator.items.len(),
        has_more,
        upper_ts = ?upper,
        boundary_ts = ?boundary_ts,
        boundary_open = accumulator.boundary_open,
        boundary_next_offset = ?accumulator.boundary_next_offset,
        "search date pipeline finished"
    );
    Ok(SearchResponse {
        query: request.query,
        items: accumulator.items,
        pagination,
    })
}

/// Inputs for one `drain_boundary` call. Grouped to keep the function
/// signature bounded.
struct DrainBoundaryInputs<'a> {
    repository: &'a dyn SearchRepository,
    request: &'a SearchRequest,
    scope: &'a crate::AccessScope,
    accumulator: &'a mut DateAccumulator,
    raw_hits: Vec<SearchDatePointHit>,
    limit: usize,
    /// Lowercased, punctuation-split tokens from the request query; the
    /// drain applies the all-terms substring rule to `title + chunk_text`
    /// and drops non-matching records. The slice is borrowed for the
    /// duration of one call so the pipeline can re-use the same
    /// `keyword_terms` across the whole walk.
    keyword_terms: &'a [String],
    /// Lowercased literal phrase used when `keyword_terms` is empty
    /// (e.g. the query is only punctuation / dashes). The drain falls
    /// back to `lower(title) LIKE phrase OR lower(chunk) LIKE phrase`
    /// so the date pipeline matches the keyword path's behaviour
    /// byte-for-byte.
    keyword_phrase: &'a str,
}

/// Hydrate one batch and push the boundary records (in Qdrant scroll
/// return order) into the accumulator. The Qdrant order is the
/// authoritative ordering — the pipeline does NOT re-sort in memory, so a
/// reissued call with the same offset returns the same records in the same
/// order. The in-memory `seen` set is the only dedup mechanism.
async fn drain_boundary(inputs: DrainBoundaryInputs<'_>) -> Result<bool> {
    let DrainBoundaryInputs {
        repository,
        request,
        scope,
        accumulator,
        raw_hits,
        limit,
        keyword_terms,
        keyword_phrase,
    } = inputs;
    if raw_hits.is_empty() {
        return Ok(false);
    }
    let chunk_ids: Vec<uuid::Uuid> = raw_hits.iter().map(|hit| hit.chunk_id).collect();
    let hydrated = repository
        .fetch_search_hits_by_chunk_ids(&chunk_ids, request, scope)
        .await?;
    let mut window_items: Vec<SearchHit> = Vec::with_capacity(raw_hits.len());
    for raw in raw_hits {
        if let Some(mut hit) = hydrated.get(&raw.chunk_id).cloned() {
            hit.score = raw.score;
            hit.vector_score = Some(raw.score);
            if hit.published_at.is_none()
                && let Some(ts) = raw.published_ts
            {
                hit.published_at = DateTime::<Utc>::from_timestamp(ts, 0);
            }
            window_items.push(hit);
        }
    }
    window_items.retain(|hit| {
        super::is_meaningful_text(&hit.chunk_text)
            && super::metadata_filters_match(&hit.metadata_json, request)
            && matches_keyword_terms(hit, keyword_terms, keyword_phrase)
    });
    // The Qdrant `scroll` return order is the authoritative ordering for
    // the boundary. The pipeline keeps that order verbatim so a reissued
    // call with the same offset returns the same records in the same
    // order; the in-memory `seen` set is the only dedup mechanism.
    let mut any_pushed = false;
    for hit in window_items {
        if accumulator.items.len() >= limit {
            break;
        }
        if accumulator.push(hit) {
            any_pushed = true;
            accumulator.pushed_in_run = true;
        }
    }
    Ok(any_pushed)
}

async fn fetch_window(
    index: &dyn SearchIndex,
    vector: Vec<f32>,
    request: &SearchRequest,
    scope: &crate::AccessScope,
    before: Option<i64>,
    limit: usize,
) -> Result<DateWindowPage> {
    let started = std::time::Instant::now();
    let page = index
        .search_by_date_window(
            vector,
            request,
            scope,
            DateWindowQuery {
                after: DateBound::Unbounded,
                before: before.map(DateBound::Inclusive).unwrap_or(DateBound::Unbounded),
                limit,
                offset: None,
            },
        )
        .await?;
    info!(
        fetched = page.hits.len(),
        before = ?before,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "search date cross-timestamp window fetched"
    );
    Ok(page)
}

async fn fetch_boundary(
    index: &dyn SearchIndex,
    vector: Vec<f32>,
    request: &SearchRequest,
    scope: &crate::AccessScope,
    boundary_ts: i64,
    offset: Option<uuid::Uuid>,
    limit: usize,
) -> Result<crate::DateWindowPage> {
    let started = std::time::Instant::now();
    // The per-timestamp drain filters to `published_ts == boundary_ts`
    // and pages through the population via the Qdrant-side offset. The
    // pipeline carries the previous window's `next_offset` (or the
    // cursor's `offset`) as the next call's `offset` so a replayed
    // request resumes the boundary exactly where the previous one
    // stopped. The Qdrant return order is deterministic per ordering
    // epoch, so threading the offset is the resume mechanism.
    let page = index
        .search_by_date_window(
            vector,
            request,
            scope,
            DateWindowQuery {
                after: DateBound::Unbounded,
                before: DateBound::Inclusive(boundary_ts),
                limit,
                offset,
            },
        )
        .await?;
    let DateWindowPage { hits, next_offset } = page;
    let hits: Vec<SearchDatePointHit> = hits
        .into_iter()
        .filter(|hit| hit.published_ts == Some(boundary_ts))
        .collect();
    info!(
        boundary_ts,
        fetched = hits.len(),
        has_next_offset = next_offset.is_some(),
        elapsed_ms = started.elapsed().as_millis() as u64,
        "search date boundary window fetched"
    );
    Ok(DateWindowPage { hits, next_offset })
}

async fn embed_once(
    embedding: &dyn SearchEmbeddingProvider,
    request: &SearchRequest,
) -> Result<Vec<f32>> {
    let embed_started = std::time::Instant::now();
    let vector = embedding.embed_query(&request.query).await?;
    info!(
        elapsed_ms = embed_started.elapsed().as_millis() as u64,
        "search date pipeline embedded query"
    );
    Ok(vector)
}

fn validate_date_context(date: &DateCursor, context: &CursorContext) -> Result<()> {
    if date.query_hash != context.query_hash {
        return Err(anyhow!(
            "cursor context mismatch: the cursor belongs to a different query; start again from the first page"
        ));
    }
    if date.filter_hash != context.filter_hash {
        return Err(anyhow!(
            "cursor context mismatch: filters (locale, source, group, date range, or metadata filters) changed; start again from the first page"
        ));
    }
    if date.settings_hash != context.settings_hash {
        return Err(anyhow!(
            "cursor context mismatch: search settings changed; start again from the first page"
        ));
    }
    if date.generation != context.generation {
        return Err(anyhow!(
            "cursor context mismatch: the indexed data has been refreshed; start again from the first page"
        ));
    }
    if date.limit != context.limit {
        return Err(anyhow!(
            "cursor context mismatch: page size changed from {} to {}; start again from the first page",
            date.limit,
            context.limit
        ));
    }
    // The request-level `MAX_DATE_WINDOWS` cap is per-request: the
    // cursor no longer carries a `windows_consumed` counter, so a
    // replayed request always starts with a fresh budget and there is
    // no over-64 reject-on-resume path.
    let _ = MAX_DATE_WINDOWS;
    Ok(())
}
