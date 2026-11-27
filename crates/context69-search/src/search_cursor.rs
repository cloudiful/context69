//! Opaque search pagination cursor encoding and ordering-epoch validation.
//!
//! Context69 issues compact `cp1.` tokens through the shared
//! `cloudiful-pagination` codec: the prefix `cp1.` followed by unpadded
//! URL-safe Base64 of `[kind: u8][binding: 32 bytes][payload]`. The token
//! stays opaque for clients; the binding is a full SHA-256 fingerprint of
//! the request context the cursor was issued for, and the payload carries
//! the business state the pipeline needs to resume.
//!
//! Two cursor families exist and never accept each other:
//!
//! - relevance (`kind = 1`): the candidate-window offset, the page size and
//!   the ordering epoch (local or rerank).
//! - date (`kind = 2`): the keyset walk state over `published_ts DESC`
//!   (`before`, `boundary_ts`, the Qdrant-side resume offset and the
//!   snapshot `upper` ceiling).
//!
//! The binding is domain-separated per family and covers the query, filter,
//! settings, generation and page-size context, so a cursor is only valid
//! inside the context that issued it. A mismatch aborts the request with a
//! generic context-mismatch diagnostic: the combined digest cannot name the
//! field that changed, so the client is told to restart from the first page.
//!
//! The former `c1:` / `c2:` / `c3:` / `c4:` formats are gone. They are
//! rejected with an explicit restart-first-page error so a client holding an
//! old cursor never replays a stale window shape.

use anyhow::Result;
use cloudiful_pagination::Binding;
use context69_contracts_core::errors::DomainError;
use context69_contracts_search::SearchHit;
use context69_contracts_search::search::SearchPagination;

use super::{
    MAX_SEARCH_CANDIDATE_WINDOW, derive_match_reason, is_meaningful_text, resolve_search_window,
};
use crate::SearchCache;

/// Prefix of the current cursor family. The shared codec owns the framing;
/// this prefix is the only part of the wire format Context69 decides.
const CURSOR_PREFIX: &str = "cp1.";
/// Retired cursor prefixes. They are recognised only to fail with an
/// explicit restart-first-page error.
const LEGACY_CURSOR_PREFIXES: [&str; 4] = ["c1:", "c2:", "c3:", "c4:"];
/// Cursor kind for the relevance family.
const CURSOR_KIND_RELEVANCE: u8 = 1;
/// Cursor kind for the date family.
const CURSOR_KIND_DATE: u8 = 2;
/// Binding domain of the relevance family.
const RELEVANCE_BINDING_DOMAIN: &str = "context69/search/relevance";
/// Binding domain of the date family.
const DATE_BINDING_DOMAIN: &str = "context69/search/date";
/// Fixed relevance payload: `[mode: u8][offset: u64][limit: u64]`.
const RELEVANCE_PAYLOAD_LEN: usize = 17;
/// Fixed date payload: `[flags: u8][limit: u64][before: i64][boundary_ts:
/// i64][upper: i64][offset: 16 bytes]`.
const DATE_PAYLOAD_LEN: usize = 49;
/// Date payload flag: the `before` bound is present.
const DATE_FLAG_BEFORE: u8 = 0b0001;
/// Date payload flag: the `boundary_ts` is present.
const DATE_FLAG_BOUNDARY_TS: u8 = 0b0010;
/// Date payload flag: the snapshot `upper` ceiling is present.
const DATE_FLAG_UPPER: u8 = 0b0100;
/// Date payload flag: the Qdrant-side resume offset is present.
const DATE_FLAG_OFFSET: u8 = 0b1000;
/// Every date payload flag this version knows.
const DATE_FLAGS_KNOWN: u8 =
    DATE_FLAG_BEFORE | DATE_FLAG_BOUNDARY_TS | DATE_FLAG_UPPER | DATE_FLAG_OFFSET;
/// Upper bound on the number of date-mode windows one request can walk. The
/// request-level cap is per-request: the cursor does not accumulate the
/// window counter across requests, so a replayed request always starts with a
/// fresh budget. When the cap fires mid-walk the pipeline emits a resumable
/// cursor + `has_more = true` so the next request can continue the walk.
pub(crate) const MAX_DATE_WINDOWS: usize = 64;
/// Inclusive hard ceiling on `published_ts` snapshots we accept. Keeps the
/// cursor payload bounded and rejects adversarial cursors that try to encode a
/// near-i64::MAX timestamp.
const DATE_TIMESTAMP_CAP: i64 = 32_503_680_000; // year 3000 in seconds

/// Rerank / local mode the cursor pins. Stored separately from
/// `rerank_applied` so the wire format stays self-describing when the caller
/// knows the request mode but not yet whether rerank produced the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CursorMode {
    Local,
    Rerank,
}

impl CursorMode {
    fn code(self) -> u8 {
        match self {
            Self::Local => 0,
            Self::Rerank => 1,
        }
    }

    fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Local),
            1 => Some(Self::Rerank),
            _ => None,
        }
    }
}

/// Build a context binding for a request. Both POST and SSE use the same
/// function so the cursor and the active request share an identical context.
pub(crate) fn cursor_context(
    request: &context69_contracts_search::SearchRequest,
    limit: usize,
    generation: i64,
    settings: &crate::SearchSettings,
) -> CursorContext {
    let query_hash = SearchCache::query_hash(&request.query);
    let filter_hash = SearchCache::filter_hash(request);
    let settings_hash = SearchCache::settings_hash(settings);
    CursorContext {
        query_hash,
        filter_hash,
        generation,
        settings_hash,
        limit,
    }
}

/// Identifies the request context a cursor must match. The hashes cover
/// anything that would change the candidate set or the rerank ordering, and
/// the `limit` blocks a page-1 cursor (limit=8) from being replayed against
/// a page-1 limit=16 request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CursorContext {
    pub query_hash: String,
    pub filter_hash: String,
    pub generation: i64,
    pub settings_hash: String,
    pub limit: usize,
}

/// The full-context binding of a relevance cursor. The parts are the ordered
/// request context, so any change to query, filters, settings, generation or
/// page size produces a different binding.
fn relevance_binding(context: &CursorContext) -> Binding {
    let generation = context.generation.to_be_bytes();
    let limit = context.limit.to_be_bytes();
    Binding::new(
        RELEVANCE_BINDING_DOMAIN,
        &[
            context.query_hash.as_bytes(),
            context.filter_hash.as_bytes(),
            context.settings_hash.as_bytes(),
            &generation,
            &limit,
        ],
    )
}

/// The full-context binding of a date cursor. The domain differs from the
/// relevance family, so a relevance cursor can never satisfy a date request
/// and the other way round.
fn date_binding(context: &CursorContext) -> Binding {
    let generation = context.generation.to_be_bytes();
    let limit = context.limit.to_be_bytes();
    Binding::new(
        DATE_BINDING_DOMAIN,
        &[
            context.query_hash.as_bytes(),
            context.filter_hash.as_bytes(),
            context.settings_hash.as_bytes(),
            &generation,
            &limit,
        ],
    )
}

/// Binding of a date cursor, exposed to the date pipeline which validates
/// its own context.
pub(crate) fn date_cursor_binding(context: &CursorContext) -> Binding {
    date_binding(context)
}

/// Decoded relevance cursor: the window offset, the page size, the ordering
/// epoch and the context binding the cursor was issued under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RelevanceCursor {
    pub mode: CursorMode,
    pub rerank_applied: bool,
    pub offset: usize,
    pub limit: usize,
    pub binding: Binding,
}

/// The business state of a date cursor: the keyset walk position over
/// `published_ts DESC`.
///
/// `before` is the inclusive upper edge of the next cross-timestamp fetch
/// (`published_ts <= before`); the boundary timestamp, when set, is drained
/// by repeated `published_ts == boundary_ts` calls using the Qdrant-side
/// `next_page_offset` (the `offset` field). `upper` is the snapshot
/// `published_ts` ceiling from the first page and bounds wall-clock drift on
/// replay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DateCursorState {
    pub limit: usize,
    /// Inclusive upper edge of the next cross-timestamp fetch.
    /// `None` means the next cross-ts fetch is unbounded above.
    pub before: Option<i64>,
    /// Timestamp currently being drained (in-flight same-second walk).
    /// `None` means no boundary in flight: the next request must start a
    /// fresh cross-timestamp fetch bounded by `before`.
    pub boundary_ts: Option<i64>,
    /// Qdrant-side keyset resume key for the per-timestamp drain. `None`
    /// means the Qdrant side reported no further records at `boundary_ts`
    /// in the previous window. The next request passes the offset straight
    /// back to Qdrant as the `offset` argument so the same ordering epoch
    /// returns the records after the last consumed one.
    pub offset: Option<uuid::Uuid>,
    /// Wall-clock bound on `published_ts` captured on the first request.
    /// All cross-timestamp fetches must keep `published_ts <= upper` so a
    /// replayed request does not pick up records indexed after the first
    /// page. `None` means the index reported no visible upper bound.
    pub upper: Option<i64>,
}

/// Decoded date cursor: the keyset walk state plus the context binding the
/// cursor was issued under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DateCursor {
    pub state: DateCursorState,
    pub binding: Binding,
}

/// Encode `(offset, rerank_applied, context)` into the opaque relevance
/// cursor string.
pub(crate) fn encode_cursor(
    offset: usize,
    rerank_applied: bool,
    limit: usize,
    context: &CursorContext,
) -> String {
    let mode = if rerank_applied {
        CursorMode::Rerank
    } else {
        CursorMode::Local
    };
    let mut payload = Vec::with_capacity(RELEVANCE_PAYLOAD_LEN);
    payload.push(mode.code());
    payload.extend_from_slice(&u64::try_from(offset).unwrap_or(u64::MAX).to_be_bytes());
    payload.extend_from_slice(&u64::try_from(limit).unwrap_or(u64::MAX).to_be_bytes());
    encode_payload(CURSOR_KIND_RELEVANCE, &relevance_binding(context), &payload)
}

/// Encode the date cursor state into the opaque date form. The `before` and
/// `upper` timestamps are bounded by `DATE_TIMESTAMP_CAP`; `before == None`
/// means "open above" (the search runs without an upper `published_ts`
/// ceiling).
pub(crate) fn encode_date_cursor(context: &CursorContext, state: &DateCursorState) -> String {
    let mut payload = Vec::with_capacity(DATE_PAYLOAD_LEN);
    let mut flags = 0u8;
    if state.before.is_some() {
        flags |= DATE_FLAG_BEFORE;
    }
    if state.boundary_ts.is_some() {
        flags |= DATE_FLAG_BOUNDARY_TS;
    }
    if state.upper.is_some() {
        flags |= DATE_FLAG_UPPER;
    }
    if state.offset.is_some() {
        flags |= DATE_FLAG_OFFSET;
    }
    payload.push(flags);
    payload.extend_from_slice(&u64::try_from(state.limit).unwrap_or(u64::MAX).to_be_bytes());
    payload.extend_from_slice(&bound_ts(state.before).to_be_bytes());
    payload.extend_from_slice(&bound_ts(state.boundary_ts).to_be_bytes());
    payload.extend_from_slice(&bound_ts(state.upper).to_be_bytes());
    let offset = state.offset.unwrap_or(uuid::Uuid::nil());
    payload.extend_from_slice(offset.as_bytes());
    encode_payload(CURSOR_KIND_DATE, &date_binding(context), &payload)
}

/// Encode one cursor family. The payload layouts are fixed width, so the
/// encoded token always fits the shared cursor limit.
fn encode_payload(kind: u8, binding: &Binding, payload: &[u8]) -> String {
    cloudiful_pagination::encode(kind, binding, payload)
        .expect("fixed-width cursor payload fits the shared cursor limit")
}

fn bound_ts(value: Option<i64>) -> i64 {
    match value {
        None => 0,
        Some(value) if value < 0 => 0,
        Some(value) if value > DATE_TIMESTAMP_CAP => DATE_TIMESTAMP_CAP,
        Some(value) => value,
    }
}

/// Decode any cursor (relevance or date) into its typed form.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum DecodedCursor {
    Relevance(RelevanceCursor),
    Date(DateCursor),
}

pub(crate) fn decode_cursor(raw: &str) -> Result<DecodedCursor> {
    // The retired prefixes are recognised before the shared decoder so an
    // old cursor — which is longer than the shared limit — still fails with
    // the restart-first-page instruction instead of a length error.
    if let Some(prefix) = LEGACY_CURSOR_PREFIXES
        .iter()
        .find(|prefix| raw.starts_with(**prefix))
    {
        return Err(DomainError::invalid_argument(format!(
            "cursor format '{prefix}' is no longer supported; restart the search from the first page"
        ))
        .into());
    }
    if !raw.starts_with(CURSOR_PREFIX) {
        return Err(DomainError::invalid_argument("unknown cursor prefix or version").into());
    }
    let decoded = cloudiful_pagination::decode(raw).map_err(|error| match error {
        cloudiful_pagination::CursorError::Malformed => {
            DomainError::invalid_argument("cursor payload is malformed")
        }
        cloudiful_pagination::CursorError::TooLong => DomainError::invalid_argument(
            "cursor exceeds the maximum supported length; restart the search from the first page",
        ),
        cloudiful_pagination::CursorError::UnsupportedVersion => {
            DomainError::invalid_argument("unknown cursor prefix or version")
        }
        cloudiful_pagination::CursorError::WrongKind => {
            DomainError::invalid_argument("cursor kind does not match the expected ordering")
        }
        cloudiful_pagination::CursorError::BindingMismatch => DomainError::invalid_argument(
            "cursor context mismatch: the cursor belongs to a different search context; start again from the first page",
        ),
    })?;
    match decoded.kind {
        CURSOR_KIND_RELEVANCE => Ok(DecodedCursor::Relevance(decode_relevance_payload(
            &decoded,
        )?)),
        CURSOR_KIND_DATE => Ok(DecodedCursor::Date(decode_date_payload(&decoded)?)),
        other => Err(DomainError::invalid_argument(format!(
            "cursor kind {other} is not a supported search cursor"
        ))
        .into()),
    }
}

fn decode_relevance_payload(
    decoded: &cloudiful_pagination::DecodedCursor,
) -> Result<RelevanceCursor> {
    if decoded.payload.len() != RELEVANCE_PAYLOAD_LEN {
        return Err(DomainError::invalid_argument("cursor payload is malformed").into());
    }
    let payload = &decoded.payload;
    let mode = CursorMode::from_code(payload[0]).ok_or_else(|| {
        DomainError::invalid_argument("cursor payload has an unknown ordering mode")
    })?;
    let offset = u64::from_be_bytes(payload[1..9].try_into().expect("fixed width"));
    let limit = u64::from_be_bytes(payload[9..17].try_into().expect("fixed width"));
    let offset = usize::try_from(offset)
        .map_err(|_| DomainError::invalid_argument("cursor offset is out of range"))?;
    let limit = usize::try_from(limit)
        .map_err(|_| DomainError::invalid_argument("cursor page size is out of range"))?;
    if !(1..=100).contains(&limit) {
        return Err(DomainError::invalid_argument(format!(
            "cursor page size {limit} is outside the allowed 1..=100 range"
        ))
        .into());
    }
    if offset > MAX_SEARCH_CANDIDATE_WINDOW {
        return Err(DomainError::invalid_argument(format!(
            "cursor offset {offset} exceeds the searchable candidate window"
        ))
        .into());
    }
    Ok(RelevanceCursor {
        mode,
        rerank_applied: matches!(mode, CursorMode::Rerank),
        offset,
        limit,
        binding: decoded.binding,
    })
}

fn decode_date_payload(decoded: &cloudiful_pagination::DecodedCursor) -> Result<DateCursor> {
    if decoded.payload.len() != DATE_PAYLOAD_LEN {
        return Err(DomainError::invalid_argument("date cursor payload is malformed").into());
    }
    let payload = &decoded.payload;
    let flags = payload[0];
    if flags & !DATE_FLAGS_KNOWN != 0 {
        return Err(DomainError::invalid_argument(
            "date cursor payload carries unknown state flags",
        )
        .into());
    }
    let limit = u64::from_be_bytes(payload[1..9].try_into().expect("fixed width"));
    let limit = usize::try_from(limit)
        .map_err(|_| DomainError::invalid_argument("date cursor page size is too large"))?;
    if !(1..=100).contains(&limit) {
        return Err(DomainError::invalid_argument(format!(
            "date cursor page size {limit} is outside the allowed 1..=100 range"
        ))
        .into());
    }
    let before = decode_ts(payload, 9, flags & DATE_FLAG_BEFORE != 0, "before")?;
    let boundary_ts = decode_ts(
        payload,
        17,
        flags & DATE_FLAG_BOUNDARY_TS != 0,
        "boundary_ts",
    )?;
    let upper = decode_ts(payload, 25, flags & DATE_FLAG_UPPER != 0, "upper")?;
    let offset = if flags & DATE_FLAG_OFFSET != 0 {
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(&payload[33..49]);
        Some(uuid::Uuid::from_bytes(bytes))
    } else {
        None
    };
    Ok(DateCursor {
        state: DateCursorState {
            limit,
            before,
            boundary_ts,
            offset,
            upper,
        },
        binding: decoded.binding,
    })
}

/// Read one optional timestamp from the fixed date payload.
fn decode_ts(payload: &[u8], start: usize, present: bool, field: &str) -> Result<Option<i64>> {
    if !present {
        return Ok(None);
    }
    let value = i64::from_be_bytes(payload[start..start + 8].try_into().expect("fixed width"));
    if !(0..=DATE_TIMESTAMP_CAP).contains(&value) {
        return Err(
            DomainError::invalid_argument(format!("date cursor {field} is out of range")).into(),
        );
    }
    Ok(Some(value))
}

/// Reject a relevance cursor whose ordering epoch disagrees with the
/// ordering the current pipeline actually produced. A missing cursor is
/// always accepted. Date cursors are rejected outright (the date pipeline
/// uses `validate_date_cursor` instead) and relevance cursors are rejected
/// when the date pipeline runs.
pub(crate) fn ensure_cursor_ordering(raw_cursor: Option<&str>, rerank_applied: bool) -> Result<()> {
    let Some(raw) = raw_cursor else {
        return Ok(());
    };
    let cursor = decode_cursor(raw)?;
    match cursor {
        DecodedCursor::Date(_) => Err(DomainError::invalid_argument(
            "cursor ordering mismatch: the cursor belongs to the date ordering; date cursors are only valid against `sort=date` requests"
        ).into()),
        DecodedCursor::Relevance(rel) => {
            if rel.rerank_applied == rerank_applied {
                return Ok(());
            }
            let epoch = |reranked: bool| if reranked { "reranked" } else { "local" };
            Err(DomainError::invalid_argument(format!(
                "cursor ordering mismatch: the cursor belongs to the {} ordering but the current search resolves to the {} ordering; start again from the first page",
                epoch(rel.rerank_applied),
                epoch(rerank_applied)
            ))
            .into())
        }
    }
}

/// The one generic context-mismatch diagnostic. The combined binding cannot
/// name the changed context field, and an edited payload page size is the
/// same kind of context disagreement, so both failures share this text.
pub(crate) fn cursor_context_mismatch_error() -> anyhow::Error {
    DomainError::invalid_argument(
        "cursor context mismatch: the cursor belongs to a different search context; start again from the first page",
    )
    .into()
}

/// Reject a payload page size that disagrees with the effective request page
/// size.
///
/// The binding covers the request context page size, but the payload page
/// size is plain bytes inside the unkeyed payload: the binding is a
/// fingerprint, not a MAC, so an edited payload limit keeps matching it.
/// Accepting such a cursor would let the payload choose the page geometry
/// (and, for relevance, the alignment divisor) independently of the request.
/// Both cursor families and both context validators must call this.
pub(crate) fn validate_cursor_payload_limit(
    payload_limit: usize,
    context: &CursorContext,
) -> Result<()> {
    if payload_limit != context.limit {
        return Err(cursor_context_mismatch_error());
    }
    Ok(())
}

/// Reject an offset that is not aligned to `limit`. The relevance pagination
/// contract only ever emits multiples of the page size, so a misaligned
/// offset means a malformed client or a cursor that no longer fits the
/// current page-size geometry.
fn validate_offset_alignment(offset: usize, limit: usize) -> Result<()> {
    if !offset.is_multiple_of(limit) {
        return Err(DomainError::invalid_argument(format!(
            "cursor offset {offset} is not aligned to the page size {limit}; start again from the first page"
        ))
        .into());
    }
    Ok(())
}

/// Reject a cursor whose context (query / filter / page size / generation /
/// settings) disagrees with the request.
///
/// Every context field is covered by one combined binding and the diagnostic
/// is generic: the changed field cannot be recovered from the digest and is
/// not guessed. The payload page size is compared with the effective request
/// page size first, because the binding does not cover the payload copy, and
/// the relevance alignment divisor is then the effective request page size
/// rather than an independently accepted payload value.
pub(crate) fn validate_cursor_context(
    raw_cursor: Option<&str>,
    context: &CursorContext,
) -> Result<()> {
    let Some(raw) = raw_cursor else {
        return Ok(());
    };
    let cursor = decode_cursor(raw)?;
    match &cursor {
        DecodedCursor::Relevance(rel) => {
            validate_cursor_payload_limit(rel.limit, context)?;
            if rel.binding != relevance_binding(context) {
                return Err(cursor_context_mismatch_error());
            }
            validate_offset_alignment(rel.offset, context.limit)?;
        }
        DecodedCursor::Date(date) => {
            validate_cursor_payload_limit(date.state.limit, context)?;
            if date.binding != date_binding(context) {
                return Err(cursor_context_mismatch_error());
            }
        }
    }
    Ok(())
}

/// Reject a relevance cursor whose offset is not aligned to its payload page
/// size. This is the pre-context geometry gate: once a cursor's context is
/// accepted, its payload page size equals the effective request page size
/// (`validate_cursor_context`), so the same divisor applies to both. Date
/// cursors are ignored (alignment is governed by the windowing logic).
pub(crate) fn validate_cursor_alignment(raw_cursor: Option<&str>) -> Result<()> {
    let Some(raw) = raw_cursor else {
        return Ok(());
    };
    let cursor = decode_cursor(raw)?;
    match cursor {
        DecodedCursor::Date(_) => Ok(()),
        DecodedCursor::Relevance(rel) => validate_offset_alignment(rel.offset, rel.limit),
    }
}

/// Reject a cursor that is incompatible with the date pipeline: only date
/// cursors are accepted; relevance cursors are rejected with a clean error
/// so the client can restart from the first page.
pub(crate) fn validate_date_cursor(raw_cursor: Option<&str>) -> Result<DateCursor> {
    let Some(raw) = raw_cursor else {
        return Err(DomainError::invalid_argument(
            "date cursor missing: date-mode requests must carry a cursor or be a fresh first page",
        )
        .into());
    };
    match decode_cursor(raw)? {
        DecodedCursor::Date(date) => Ok(date),
        DecodedCursor::Relevance(_) => Err(DomainError::invalid_argument(
            "cursor ordering mismatch: the cursor belongs to the relevance ordering; relevance cursors are not valid for `sort=date` requests"
        ).into()),
    }
}

/// Window geometry shared by the ordering stages of one request.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PageWindow {
    pub offset: usize,
    pub limit: usize,
    pub requested_limit: usize,
    pub fetch_limit: usize,
    pub can_probe_window: bool,
    pub upstream_capped: bool,
}

/// Finalize an ordered result list (local or reranked) into its page window:
/// truncate to `fetch_limit`, retain meaningful text, derive match reasons,
/// resolve the `has_more`/total signals, slice `[offset, offset + limit)` and
/// build the ordering-epoch pagination with cursors.
pub(crate) fn finalize_window(
    mut results: Vec<SearchHit>,
    window: &PageWindow,
    context: &CursorContext,
    rerank_applied: bool,
) -> Result<(SearchPagination, Vec<SearchHit>)> {
    results.truncate(window.fetch_limit);
    results.retain(|hit| is_meaningful_text(&hit.chunk_text));

    // Derive the canonical match reason before the response is served so both
    // live and cached payloads carry the display-ready value.
    for hit in &mut results {
        hit.match_reason = Some(derive_match_reason(
            hit.vector_score,
            hit.match_reason.as_deref(),
        ));
    }

    // Compute `has_more` before slicing the current page so the probe item
    // beyond `offset + limit` is not lost to `take`.
    let collected_len = results.len();
    let (total_window, has_more) = resolve_search_window(
        collected_len,
        window.requested_limit,
        window.can_probe_window,
        window.upstream_capped,
    );
    let total = u64::try_from(total_window)?;
    let items = results
        .into_iter()
        .skip(window.offset)
        .take(window.limit)
        .collect::<Vec<_>>();
    let pagination = build_search_pagination(
        window.offset,
        window.limit,
        total,
        has_more,
        rerank_applied,
        context,
    )?;
    Ok((pagination, items))
}

/// Build search-window pagination carrying the cursors of the page's ordering
/// epoch. `page` is derived from the absolute offset so cursor and legacy page
/// navigation stay consistent (`offset == (page - 1) * page_size`).
pub(crate) fn build_search_pagination(
    offset: usize,
    limit: usize,
    total: u64,
    has_more: Option<bool>,
    rerank_applied: bool,
    context: &CursorContext,
) -> Result<SearchPagination> {
    if limit == 0 {
        return Err(DomainError::invalid_argument("page_size must be between 1 and 100").into());
    }
    let page = u32::try_from(offset / limit + 1)?;
    let page_size = u32::try_from(limit)?;
    let mut pagination = SearchPagination::try_new_search_window(page, page_size, total, has_more)?;
    pagination.next_cursor = next_cursor_of(offset, limit, has_more, rerank_applied, context);
    pagination.prev_cursor = prev_cursor_of(offset, limit, rerank_applied, context);
    Ok(pagination)
}

/// Cursor for the next window: present only when the server positively knows
/// the next page is reachable (probe observed more candidates beyond this
/// page) AND the next offset would still fit inside the candidate window.
/// An unknown `has_more` (because the window reached the cap or an upstream
/// fetch saturated its top-K) intentionally omits the cursor so the client
/// never receives one that the next request would reject.
fn next_cursor_of(
    offset: usize,
    limit: usize,
    has_more: Option<bool>,
    rerank_applied: bool,
    context: &CursorContext,
) -> Option<String> {
    if has_more != Some(true) {
        return None;
    }
    let next_offset = offset.saturating_add(limit);
    if next_offset >= MAX_SEARCH_CANDIDATE_WINDOW {
        return None;
    }
    Some(encode_cursor(next_offset, rerank_applied, limit, context))
}

/// Cursor for the previous window, present whenever the current page is not
/// the first.
fn prev_cursor_of(
    offset: usize,
    limit: usize,
    rerank_applied: bool,
    context: &CursorContext,
) -> Option<String> {
    if offset == 0 {
        return None;
    }
    Some(encode_cursor(
        offset.saturating_sub(limit),
        rerank_applied,
        limit,
        context,
    ))
}

/// Rewrite the fixed payload of an already encoded cursor while keeping its
/// kind and binding.
///
/// The pure regression tests use this to model a client that edits the
/// unkeyed payload bytes of a token it received; the 32-byte binding is
/// deliberately retained, because it is a fingerprint and not a MAC.
#[cfg(test)]
pub(crate) fn tamper_payload(raw: &str, mutate: impl FnOnce(&mut Vec<u8>)) -> String {
    let decoded = cloudiful_pagination::decode(raw).expect("valid cursor decodes");
    let mut payload = decoded.payload;
    mutate(&mut payload);
    cloudiful_pagination::encode(decoded.kind, &decoded.binding, &payload)
        .expect("fixed-width tampered payload still encodes")
}

#[cfg(test)]
mod tests {
    use super::{
        CursorContext, DATE_TIMESTAMP_CAP, MAX_DATE_WINDOWS, MAX_SEARCH_CANDIDATE_WINDOW,
        RelevanceCursor, decode_cursor, encode_cursor, encode_date_cursor, ensure_cursor_ordering,
        validate_cursor_alignment, validate_cursor_context, validate_date_cursor,
    };
    use super::{DateCursor, DateCursorState, DecodedCursor};

    fn ctx(limit: usize) -> CursorContext {
        CursorContext {
            query_hash: "qh".to_string(),
            filter_hash: "fh".to_string(),
            generation: 7,
            settings_hash: "sh".to_string(),
            limit,
        }
    }

    fn sample_date_state() -> DateCursorState {
        DateCursorState {
            limit: 8,
            before: Some(1_700_000_000),
            boundary_ts: Some(1_699_999_990),
            offset: Some(uuid::Uuid::nil()),
            upper: Some(1_700_000_000),
        }
    }

    #[test]
    fn cursor_round_trips_both_orderings_and_offsets() {
        for (offset, rerank_applied) in [(0, false), (8, true), (40, false), (999, true)] {
            let raw = encode_cursor(offset, rerank_applied, 8, &ctx(8));
            assert!(raw.starts_with("cp1."));
            let decoded = decode_cursor(&raw).expect("valid cursor decodes");
            assert_eq!(
                decoded,
                DecodedCursor::Relevance(RelevanceCursor {
                    rerank_applied,
                    mode: if rerank_applied {
                        super::CursorMode::Rerank
                    } else {
                        super::CursorMode::Local
                    },
                    offset,
                    limit: 8,
                    binding: super::relevance_binding(&ctx(8)),
                })
            );
        }
    }

    #[test]
    fn date_cursor_round_trip() {
        let raw = encode_date_cursor(&ctx(8), &sample_date_state());
        assert!(raw.starts_with("cp1."));
        let decoded = decode_cursor(&raw).expect("date cursor decodes");
        assert_eq!(
            decoded,
            DecodedCursor::Date(DateCursor {
                state: sample_date_state(),
                binding: super::date_binding(&ctx(8)),
            })
        );
    }

    #[test]
    fn encoded_cursors_stay_inside_the_shared_limit() {
        // The measured lengths must stay well below the 256-character input
        // cap so a cursor can travel through the MCP request surface.
        let relevance = encode_cursor(1_999, true, 100, &ctx(100));
        let date = encode_date_cursor(&ctx(100), &sample_date_state());
        for token in [&relevance, &date] {
            assert!(token.len() <= 256, "token too long: {}", token.len());
            assert!(
                token
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_')),
                "token must be ASCII and URL-safe"
            );
        }
        assert_eq!(relevance.len(), 71, "measured relevance token length");
        assert_eq!(date.len(), 114, "measured date token length");
    }

    #[test]
    fn malformed_cursors_are_rejected() {
        assert!(decode_cursor("cp1.zz").is_err(), "bad base64 rejected");
        assert!(decode_cursor("cp1.").is_err(), "empty payload rejected");
        assert!(decode_cursor("cp1.AAAA").is_err(), "short header rejected");
        assert!(
            decode_cursor("cp1.!!!!").is_err(),
            "invalid base64 rejected"
        );
        assert!(decode_cursor("cp5.00").is_err(), "unknown prefix rejected");
        assert!(decode_cursor("").is_err());
        assert!(
            decode_cursor(&"A".repeat(300)).is_err(),
            "oversize rejected"
        );
    }

    #[test]
    fn retired_cursor_formats_are_rejected_with_restart_instructions() {
        // Every retired format fails explicitly with a restart-first-page
        // instruction; no legacy decoder remains.
        for prefix in ["c1:", "c2:", "c3:", "c4:"] {
            let raw = format!("{prefix}7b2276223a347d");
            let error = decode_cursor(&raw).expect_err("retired cursor must be rejected");
            let message = error.to_string();
            assert!(
                message.contains("no longer supported"),
                "unexpected message: {message}"
            );
            assert!(
                message.contains("first page"),
                "unexpected message: {message}"
            );
        }
    }

    #[test]
    fn oversized_cursor_offset_is_rejected() {
        let raw = encode_cursor(MAX_SEARCH_CANDIDATE_WINDOW + 1, false, 8, &ctx(8));
        assert!(decode_cursor(&raw).is_err());
    }

    #[test]
    fn date_cursor_rejects_out_of_range_timestamps() {
        // An adversarial cursor that smuggles an out-of-range timestamp must
        // be rejected at decode time.
        fn encode_raw(payload: &[u8]) -> String {
            cloudiful_pagination::encode(
                super::CURSOR_KIND_DATE,
                &super::date_binding(&ctx(8)),
                payload,
            )
            .expect("payload fits")
        }
        let mut payload = vec![0u8; super::DATE_PAYLOAD_LEN];
        payload[0] = super::DATE_FLAG_BEFORE
            | super::DATE_FLAG_BOUNDARY_TS
            | super::DATE_FLAG_UPPER
            | super::DATE_FLAG_OFFSET;
        payload[1..9].copy_from_slice(&8u64.to_be_bytes());
        for (start, value) in [
            (9usize, DATE_TIMESTAMP_CAP + 1),
            (17, DATE_TIMESTAMP_CAP + 1),
            (25, DATE_TIMESTAMP_CAP + 1),
        ] {
            let mut candidate = payload.clone();
            candidate[start..start + 8].copy_from_slice(&value.to_be_bytes());
            assert!(decode_cursor(&encode_raw(&candidate)).is_err());
        }
        // A negative timestamp is out of range as well.
        let mut candidate = payload.clone();
        candidate[9..17].copy_from_slice(&(-1i64).to_be_bytes());
        assert!(decode_cursor(&encode_raw(&candidate)).is_err());
        // Unknown state flags are rejected.
        let mut candidate = payload.clone();
        candidate[0] = 0b1111_0000;
        assert!(decode_cursor(&encode_raw(&candidate)).is_err());
    }

    #[test]
    fn date_cursor_round_trips_keyset_offset() {
        // The keyset `offset` is opaque to the client; a UUID round-trips
        // through encode/decode without changing its value.
        let mut state = sample_date_state();
        state.offset = Some(uuid::Uuid::from_u128(
            0xabcd_1234_5678_9abc_def0_1234_5678_9abc,
        ));
        let raw = encode_date_cursor(&ctx(8), &state);
        assert!(raw.starts_with("cp1."));
        let decoded = decode_cursor(&raw).expect("date cursor decodes");
        assert_eq!(
            decoded,
            DecodedCursor::Date(DateCursor {
                state: state.clone(),
                binding: super::date_binding(&ctx(8)),
            })
        );
        // `offset = None` must round-trip as `None` so the absence of the
        // field is interpreted as a no-op resume (the Qdrant side will
        // return its first page from the requested `boundary_ts` filter).
        state.offset = None;
        let raw = encode_date_cursor(&ctx(8), &state);
        let decoded = decode_cursor(&raw).expect("date cursor decodes");
        assert_eq!(
            decoded,
            DecodedCursor::Date(DateCursor {
                state: state.clone(),
                binding: super::date_binding(&ctx(8)),
            })
        );
    }

    #[test]
    fn next_cursor_is_omitted_beyond_candidate_window() {
        // A page already at the cap whose next offset would exceed the cap
        // must not surface a next cursor. The previous window is still a
        // valid offset for the prev cursor.
        let pagination = super::build_search_pagination(
            MAX_SEARCH_CANDIDATE_WINDOW - 8,
            8,
            MAX_SEARCH_CANDIDATE_WINDOW as u64,
            Some(true),
            true,
            &ctx(8),
        )
        .expect("build pagination at cap");
        assert!(pagination.next_cursor.is_none());
        // The previous window (offset - limit) is still reachable.
        assert!(pagination.prev_cursor.is_some());
    }

    #[test]
    fn unknown_has_more_does_not_emit_cursor() {
        let pagination = super::build_search_pagination(0, 8, 8, None, false, &ctx(8))
            .expect("build pagination unknown");
        assert!(pagination.next_cursor.is_none());
        assert!(pagination.prev_cursor.is_none());
    }

    #[test]
    fn cursor_ordering_mismatch_is_rejected_but_matching_epoch_passes() {
        let reranked = encode_cursor(8, true, 8, &ctx(8));
        let local = encode_cursor(8, false, 8, &ctx(8));
        // Cross-ordering use is never silently served.
        assert!(
            ensure_cursor_ordering(Some(&reranked), false).is_err(),
            "reranked cursor against local ordering rejected"
        );
        assert!(
            ensure_cursor_ordering(Some(&local), true).is_err(),
            "local cursor against reranked ordering rejected"
        );
        assert!(ensure_cursor_ordering(Some(&reranked), true).is_ok());
        assert!(ensure_cursor_ordering(Some(&local), false).is_ok());
        assert!(ensure_cursor_ordering(None, true).is_ok());
        assert!(ensure_cursor_ordering(None, false).is_ok());
        // The rejection message is client-facing and explains the epoch gap.
        let error = ensure_cursor_ordering(Some(&reranked), false).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("cursor ordering mismatch"));
        assert!(message.contains("reranked"));
        assert!(message.contains("local"));
    }

    #[test]
    fn relevance_cursor_rejected_in_date_pipeline() {
        let relevance = encode_cursor(8, true, 8, &ctx(8));
        let error = validate_date_cursor(Some(&relevance))
            .expect_err("relevance cursor must not be accepted by date pipeline");
        assert!(error.to_string().contains("relevance"));
    }

    #[test]
    fn date_cursor_rejected_in_relevance_pipeline() {
        let date = encode_date_cursor(&ctx(8), &sample_date_state());
        let error = ensure_cursor_ordering(Some(&date), false)
            .expect_err("date cursor must not be accepted by relevance pipeline");
        let message = error.to_string();
        assert!(message.contains("date"));
    }

    #[test]
    fn cursor_context_mismatch_rejects_cross_query_use() {
        let cursor = encode_cursor(8, true, 8, &ctx(8));
        let mut other = ctx(8);
        other.query_hash = "different".into();
        let error =
            validate_cursor_context(Some(&cursor), &other).expect_err("query hash must differ");
        assert!(error.to_string().contains("cursor context mismatch"));
        assert!(error.to_string().contains("first page"));
    }

    #[test]
    fn date_cursor_context_mismatch_rejects_changed_filter() {
        let cursor = encode_date_cursor(&ctx(8), &sample_date_state());
        let mut other = ctx(8);
        other.filter_hash = "different".into();
        let error =
            validate_cursor_context(Some(&cursor), &other).expect_err("filter hash must differ");
        assert!(error.to_string().contains("cursor context mismatch"));
    }

    #[test]
    fn cursor_context_mismatch_rejects_changed_limit() {
        let cursor = encode_cursor(8, true, 8, &ctx(8));
        let error = validate_cursor_context(Some(&cursor), &ctx(16))
            .expect_err("limit change must be rejected");
        let message = error.to_string();
        assert!(message.contains("cursor context mismatch"));
    }

    #[test]
    fn cursor_context_mismatch_rejects_filter_change() {
        let cursor = encode_cursor(8, true, 8, &ctx(8));
        let mut other = ctx(8);
        other.filter_hash = "different".into();
        let error =
            validate_cursor_context(Some(&cursor), &other).expect_err("filter hash must differ");
        assert!(error.to_string().contains("cursor context mismatch"));
    }

    #[test]
    fn cursor_context_mismatch_rejects_generation_bump() {
        let cursor = encode_cursor(8, true, 8, &ctx(8));
        let mut other = ctx(8);
        other.generation = 8;
        let error = validate_cursor_context(Some(&cursor), &other)
            .expect_err("generation change must be rejected");
        assert!(error.to_string().contains("cursor context mismatch"));
    }

    #[test]
    fn cursor_context_mismatch_rejects_settings_change() {
        let cursor = encode_cursor(8, true, 8, &ctx(8));
        let mut other = ctx(8);
        other.settings_hash = "different".into();
        let error = validate_cursor_context(Some(&cursor), &other)
            .expect_err("settings change must be rejected");
        assert!(error.to_string().contains("cursor context mismatch"));
    }

    #[test]
    fn cursor_context_mismatch_rejects_date_generation_bump() {
        let cursor = encode_date_cursor(&ctx(8), &sample_date_state());
        let mut other = ctx(8);
        other.generation = 9;
        let error = validate_cursor_context(Some(&cursor), &other)
            .expect_err("generation change must be rejected");
        assert!(error.to_string().contains("cursor context mismatch"));
    }

    #[test]
    fn cursor_families_do_not_share_a_binding() {
        // The relevance and date families use different binding domains, so
        // a date cursor can never satisfy a relevance request context.
        let context = ctx(8);
        assert_ne!(
            super::relevance_binding(&context),
            super::date_binding(&context)
        );
    }

    #[test]
    fn cursor_alignment_rejects_offset_not_multiple_of_limit() {
        let cursor = encode_cursor(5, false, 8, &ctx(8));
        let error = validate_cursor_alignment(Some(&cursor))
            .expect_err("misaligned cursor offset must be rejected");
        let message = error.to_string();
        assert!(message.contains("not aligned"));
        assert!(message.contains("8"));
    }

    #[test]
    fn cursor_alignment_accepts_offset_zero_with_aligned_limit() {
        let cursor = encode_cursor(0, false, 8, &ctx(8));
        assert!(validate_cursor_alignment(Some(&cursor)).is_ok());
        // limit 0 is rejected by decode itself.
        let cursor = encode_cursor(0, false, 0, &ctx(0));
        assert!(validate_cursor_alignment(Some(&cursor)).is_err());
    }

    #[test]
    fn date_cursor_alignment_ignores_geometry() {
        let cursor = encode_date_cursor(&ctx(8), &sample_date_state());
        assert!(validate_cursor_alignment(Some(&cursor)).is_ok());
    }

    #[test]
    fn tampered_relevance_payload_limit_is_rejected_by_context_validation() {
        let context = ctx(8);
        let valid = encode_cursor(8, false, 8, &context);
        assert!(validate_cursor_context(Some(&valid), &context).is_ok());

        // Keep the issued binding and rewrite only the fixed payload to
        // offset 1 / limit 1. Geometry alone accepts 1 % 1, so only the
        // payload/request page-size equality can reject this token.
        let tampered = super::tamper_payload(&valid, |payload| {
            payload[1..9].copy_from_slice(&1u64.to_be_bytes());
            payload[9..17].copy_from_slice(&1u64.to_be_bytes());
        });
        assert_ne!(tampered, valid);
        assert!(
            validate_cursor_alignment(Some(&tampered)).is_ok(),
            "the tampered geometry alone is aligned"
        );
        let error = validate_cursor_context(Some(&tampered), &context)
            .expect_err("payload page size must equal the request page size");
        assert!(error.to_string().contains("cursor context mismatch"));
        assert!(error.to_string().contains("first page"));
    }

    #[test]
    fn tampered_date_payload_limit_is_rejected_by_shared_context_validation() {
        let context = ctx(8);
        let valid = encode_date_cursor(&context, &sample_date_state());
        assert!(validate_cursor_context(Some(&valid), &context).is_ok());

        let tampered = super::tamper_payload(&valid, |payload| {
            payload[1..9].copy_from_slice(&1u64.to_be_bytes());
        });
        let error = validate_cursor_context(Some(&tampered), &context)
            .expect_err("date payload page size must equal the request page size");
        assert!(error.to_string().contains("cursor context mismatch"));
    }

    #[test]
    fn matching_payload_limits_pass_both_context_validators() {
        let context = ctx(8);
        let relevance = encode_cursor(8, true, 8, &context);
        let date = encode_date_cursor(&context, &sample_date_state());
        assert!(validate_cursor_context(Some(&relevance), &context).is_ok());
        assert!(validate_cursor_context(Some(&date), &context).is_ok());
    }

    #[test]
    fn max_date_windows_constant_is_per_request_budget() {
        // The cap is a per-request budget: replayed requests do not
        // accumulate window usage. The cursor module no longer exposes a
        // `windows_consumed` field so the rejection-at-resume behaviour
        // is impossible by construction.
        let _ = MAX_DATE_WINDOWS;
    }

    #[test]
    fn relevance_cursor_is_accepted_by_the_mcp_request_surface() {
        // An actual encoded cursor must survive the MCP request validation
        // unchanged, so a continuation request keeps working.
        let cursor = encode_cursor(8, true, 8, &ctx(8));
        let request = context69_contracts_mcp::McpSearchRequest {
            query: "rust async".to_string(),
            group_path: None,
            source_key: None,
            locale: None,
            limit: 8,
            cursor: Some(cursor.clone()),
        };
        request
            .validate()
            .expect("an issued cursor must be accepted by the MCP request surface");
        assert!(cursor.len() <= 256);
    }
}
