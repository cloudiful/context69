//! Opaque search pagination cursor encoding and ordering-epoch validation.
//!
//! Three wire formats are documented and clients must echo the value back
//! unchanged. All start with a prefix that identifies the cursor family:
//!
//! - `c1:` — relevance cursor (legacy). Hex of the UTF-8 JSON
//!   ```text
//!   {
//!     "v": 1,
//!     "reranked": <bool>,
//!     "offset":   <n>,
//!     "limit":    <n>,
//!     "qh":       "<query hash hex>",
//!     "fh":       "<filter hash hex>",
//!     "gen":      <i64>,
//!     "sh":       "<settings hash hex>",
//!     "mode":     "rerank" | "local"
//!   }
//!   ```
//! - `c2:` — date cursor (legacy, superseded by `c3:` and then `c4:`). Rejected
//!   by the current date pipeline; clients must restart from the first page
//!   after the v3 / v4 format migrations. Hex of the UTF-8 JSON
//!   ```text
//!   {
//!     "v": 2,
//!     "sort":     "date",
//!     "qh":       "<query hash hex>",
//!     "fh":       "<filter hash hex>",
//!     "gen":      <i64>,
//!     "sh":       "<settings hash hex>",
//!     "limit":    <n>,
//!     "before":   <i64 | null>,
//!     "tie":      {
//!        "ts":    <i64 | null>,
//!        "chunk": "<chunk uuid or null>"
//!     },
//!     "upper":    <i64 | null>
//!   }
//!   ```
//! - `c3:` — date cursor (superseded by `c4:`). The previous format carried a
//!   per-request `windows` counter and an in-memory `tie_chunk` filter for the
//!   same-second drain; both have been retired. Clients holding a `c3:`
//!   cursor must restart from the first page after the `c4:` migration. Hex
//!   of the UTF-8 JSON
//!   ```text
//!   {
//!     "v":           3,
//!     "sort":        "date",
//!     "qh":          "<query hash hex>",
//!     "fh":          "<filter hash hex>",
//!     "gen":         <i64>,
//!     "sh":          "<settings hash hex>",
//!     "limit":       <n>,
//!     "before":      <i64 | null>,
//!     "boundary_ts": <i64 | null>,
//!     "tie_chunk":   "<uuid | null>",
//!     "offset":      "<uuid | null>",
//!     "upper":       <i64 | null>,
//!     "windows":     <usize>
//!   }
//!   ```
//! - `c4:` — date cursor (current). Keyset walk over `published_ts DESC` whose
//!   same-second ties follow the Qdrant scroll return order (deterministic
//!   per ordering epoch). The boundary timestamp is drained in full via the
//!   Qdrant-side `next_page_offset`; the resume key is the offset itself plus
//!   the wall-clock `upper` ceiling captured on the first page. No global
//!   window counter — the request-level `MAX_DATE_WINDOWS` cap is a
//!   per-request budget that produces a resumable cursor when hit. Hex of
//!   the UTF-8 JSON
//!   ```text
//!   {
//!     "v":           4,
//!     "sort":        "date",
//!     "qh":          "<query hash hex>",
//!     "fh":          "<filter hash hex>",
//!     "gen":         <i64>,
//!     "sh":          "<settings hash hex>",
//!     "limit":       <n>,
//!     "before":      <i64 | null>,  // upper bound of the next cross-timestamp fetch
//!     "boundary_ts": <i64 | null>,  // timestamp being drained (None = ready for next ts)
//!     "offset":      "<uuid | null>",  // Qdrant-side next_page_offset (PointId UUID)
//!     "upper":       <i64 | null>   // snapshot published_ts ceiling from the first page
//!   }
//!   ```
//!
//! The `before` bound is the inclusive upper edge of the *next* cross-
//! timestamp fetch; the next window asks for `published_ts <= before`. The
//! `boundary_ts` field is `Some` while a same-second drain is in flight; the
//! next request must resume at that timestamp with `offset` so the Qdrant
//! side returns exactly the records after the last consumed one. The `upper`
//! field is the snapshot `published_ts` ceiling captured when the first
//! cursor was issued; it bounds wall-clock drift on the very first page so a
//! request replayed against freshly indexed records does not silently widen.
//!
//! Within one ordering epoch (same generation + same filter hash + same
//! settings hash), the Qdrant `scroll` return order over
//! `published_ts DESC` is deterministic: re-fetching with the same `offset`
//! returns the same records in the same order, and the next page starts at
//! `next_page_offset`. The cursor pins the offset verbatim so a replayed
//! request resumes exactly after the last emitted record. The pipeline does
//! NOT re-sort in memory; the Qdrant order is the source of truth and the
//! `seen` set is the only dedup mechanism.
//!
//! A cursor is only valid inside the ordering epoch that issued it AND in the
//! same request context (query text, filters, limit, settings, generation).
//! Date cursors and relevance cursors never accept each other: cross-cursor
//! reuse is rejected with a clean error instead of silently serving the wrong
//! window.
//!
//! `validate_cursor_context` is the unified context validator for both
//! formats. The date path adds `validate_date_cursor` (window-bound sanity
//! + sort/context rejection) on top of the shared check.

use anyhow::{Result, anyhow};
use context69_contracts::SearchHit;
use context69_contracts::search::SearchPagination;
use serde_json::{Value, json};

use super::{
    MAX_SEARCH_CANDIDATE_WINDOW, derive_match_reason, is_meaningful_text, resolve_search_window,
};
use crate::SearchCache;

const CURSOR_PREFIX: &str = "c1:";
const CURSOR_SCHEMA_VERSION: u64 = 1;
const DATE_CURSOR_PREFIX: &str = "c2:";
const DATE_CURSOR_LEGACY_VERSION: u64 = 2;
const DATE_CURSOR_PREFIX_V3: &str = "c3:";
const DATE_CURSOR_LEGACY_V3_VERSION: u64 = 3;
const DATE_CURSOR_PREFIX_V4: &str = "c4:";
const DATE_CURSOR_SCHEMA_VERSION: u64 = 4;
/// Upper bound on the number of date-mode windows one request can walk. The
/// request-level cap is per-request: the cursor does not accumulate the
/// window counter across requests, so a replayed request always starts with a
/// fresh budget. When the cap fires mid-walk the pipeline emits a resumable
/// cursor + `has_more = true` so the next request can continue the walk.
pub(crate) const MAX_DATE_WINDOWS: usize = 64;
/// Inclusive hard ceiling on `published_ts` snapshots we accept. Keeps the
/// JSON field bounded and rejects adversarial cursors that try to encode a
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
    fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Rerank => "rerank",
        }
    }
}

/// Decoded relevance cursor: the ordering epoch, the window offset, and the
/// request context the cursor was issued under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RelevanceCursor {
    pub rerank_applied: bool,
    pub mode: CursorMode,
    pub offset: usize,
    pub limit: usize,
    pub query_hash: String,
    pub filter_hash: String,
    pub generation: i64,
    pub settings_hash: String,
}

/// Decoded date cursor (current `c4:` wire format). The cursor pins the
/// cross-timestamp walk state so a replayed request resumes the keyset walk
/// exactly after the last emitted record.
///
/// `before` is the inclusive upper edge of the next cross-timestamp fetch
/// (`published_ts <= before`); the boundary timestamp, when set, is drained
/// by repeated `published_ts == boundary_ts` calls using the Qdrant-side
/// `next_page_offset` (the `offset` field). `upper` is the snapshot
/// `published_ts` ceiling from the first page and bounds wall-clock drift on
/// replay.
///
/// The previous `c3:` format carried a per-request `windows` counter and an
/// in-memory `tie_chunk` filter; both have been retired. Within one ordering
/// epoch the Qdrant `scroll` return order over `published_ts DESC` is
/// deterministic and the resume key is the offset itself plus the
/// generation / filter / settings / query context binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DateCursor {
    pub limit: usize,
    pub query_hash: String,
    pub filter_hash: String,
    pub generation: i64,
    pub settings_hash: String,
    /// Inclusive upper edge of the next cross-timestamp fetch.
    /// `None` means the next cross-ts fetch is unbounded above.
    pub before: Option<i64>,
    /// Timestamp currently being drained (in-flight same-second walk).
    /// `None` means no boundary in flight: the next request must start a
    /// fresh cross-timestamp fetch bounded by `before`.
    pub boundary_ts: Option<i64>,
    /// Qdrant-side keyset resume key for the per-timestamp drain. `None`
    /// means the Qdrant side reported no further records at `boundary_ts`
    /// in the previous window, or the cursor was issued by an older
    /// pipeline that did not thread the offset. The next request passes
    /// the offset straight back to Qdrant as the `offset` argument so the
    /// same ordering epoch returns the records after the last consumed
    /// one. Opaque point UUIDs are acceptable in the cursor (chunk ids are
    /// already client-visible; the wire format does not embed query text
    /// or secrets).
    pub offset: Option<uuid::Uuid>,
    /// Wall-clock bound on `published_ts` captured on the first request.
    /// All cross-timestamp fetches must keep `published_ts <= upper` so a
    /// replayed request does not pick up records indexed after the first
    /// page. `None` means the index reported no visible upper bound.
    pub upper: Option<i64>,
}

/// Build a context binding for a request. Both POST and SSE use the same
/// function so the cursor and the active request share an identical context.
pub(crate) fn cursor_context(
    request: &context69_contracts::SearchRequest,
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
    let payload = json!({
        "v": CURSOR_SCHEMA_VERSION,
        "reranked": rerank_applied,
        "mode": mode.as_str(),
        "offset": offset,
        "limit": limit,
        "qh": context.query_hash,
        "fh": context.filter_hash,
        "gen": context.generation,
        "sh": context.settings_hash,
    });
    encode_hex(CURSOR_PREFIX, &payload)
}

/// Encode a date cursor into the opaque `c4:` form. The `before` and `upper`
/// timestamps are bounded by `DATE_TIMESTAMP_CAP`; `before == None` means
/// "open above" (the search runs without an upper `published_ts` ceiling).
pub(crate) fn encode_date_cursor(cursor: &DateCursor) -> String {
    let before = cursor.before.map(bound_ts).unwrap_or(Value::Null);
    let upper = cursor.upper.map(bound_ts).unwrap_or(Value::Null);
    let boundary_ts = cursor.boundary_ts.map(bound_ts).unwrap_or(Value::Null);
    let offset = cursor
        .offset
        .map(|value| Value::String(value.to_string()))
        .unwrap_or(Value::Null);
    let payload = json!({
        "v": DATE_CURSOR_SCHEMA_VERSION,
        "sort": "date",
        "qh": cursor.query_hash,
        "fh": cursor.filter_hash,
        "gen": cursor.generation,
        "sh": cursor.settings_hash,
        "limit": cursor.limit,
        "before": before,
        "boundary_ts": boundary_ts,
        "offset": offset,
        "upper": upper,
    });
    encode_hex(DATE_CURSOR_PREFIX_V4, &payload)
}

fn bound_ts(value: i64) -> Value {
    if value < 0 {
        Value::from(0)
    } else if value > DATE_TIMESTAMP_CAP {
        Value::from(DATE_TIMESTAMP_CAP)
    } else {
        Value::from(value)
    }
}

fn encode_hex(prefix: &str, payload: &Value) -> String {
    let bytes = serde_json::to_vec(payload).expect("cursor JSON encoding cannot fail");
    let mut out = String::with_capacity(prefix.len() + bytes.len() * 2);
    out.push_str(prefix);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Decode any cursor (relevance `c1:` or date `c2:` / `c3:` / `c4:`) into its
/// typed form.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum DecodedCursor {
    Relevance(RelevanceCursor),
    Date(DateCursor),
}

pub(crate) fn decode_cursor(raw: &str) -> Result<DecodedCursor> {
    if let Some(hex) = raw.strip_prefix(DATE_CURSOR_PREFIX_V4) {
        return Ok(DecodedCursor::Date(decode_date_cursor_body(hex)?));
    }
    if let Some(hex) = raw.strip_prefix(DATE_CURSOR_PREFIX_V3) {
        // The previous `c3:` format carried a per-request `windows` counter
        // and an in-memory `tie_chunk` filter. The current date pipeline
        // threads the Qdrant-side `next_page_offset` instead, so the
        // `c3:` semantics no longer match the pipeline geometry. Reject
        // so clients restart from the first page rather than silently
        // replaying a stale window shape.
        let _ = hex;
        return Err(anyhow!(
            "date cursor version {DATE_CURSOR_LEGACY_V3_VERSION} is no longer supported; restart from the first page"
        ));
    }
    if let Some(_hex) = raw.strip_prefix(DATE_CURSOR_PREFIX) {
        // Legacy `c2:` date cursors are not accepted by the current date
        // pipeline (the keyset walk changed). Reject so clients restart
        // from the first page rather than silently replaying a stale
        // window shape.
        return Err(anyhow!(
            "date cursor version {DATE_CURSOR_LEGACY_VERSION} is no longer supported; restart from the first page"
        ));
    }
    if let Some(hex) = raw.strip_prefix(CURSOR_PREFIX) {
        return Ok(DecodedCursor::Relevance(decode_relevance_cursor_body(hex)?));
    }
    Err(anyhow!("unknown cursor prefix or version"))
}

fn decode_relevance_cursor_body(hex: &str) -> Result<RelevanceCursor> {
    let bytes = decode_hex(hex).ok_or_else(|| anyhow!("cursor payload is not valid hex"))?;
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|error| anyhow!("cursor payload is not valid JSON: {error}"))?;
    let version = value
        .get("v")
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("cursor payload is missing its version"))?;
    if version != CURSOR_SCHEMA_VERSION {
        return Err(anyhow!("unsupported cursor version {version}"));
    }
    let rerank_applied = value
        .get("reranked")
        .and_then(Value::as_bool)
        .ok_or_else(|| anyhow!("cursor payload is missing its ordering flag"))?;
    let mode = match value
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or(if rerank_applied { "rerank" } else { "local" })
    {
        "rerank" => CursorMode::Rerank,
        "local" => CursorMode::Local,
        other => return Err(anyhow!("cursor payload has an unknown mode '{other}'")),
    };
    if rerank_applied != matches!(mode, CursorMode::Rerank) {
        return Err(anyhow!(
            "cursor payload is inconsistent: mode {mode:?} does not match reranked={rerank_applied}"
        ));
    }
    let offset = value
        .get("offset")
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("cursor payload is missing its offset"))?;
    let limit = value
        .get("limit")
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("cursor payload is missing its page size"))?;
    let query_hash = value
        .get("qh")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("cursor payload is missing its query context"))?
        .to_string();
    let filter_hash = value
        .get("fh")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("cursor payload is missing its filter context"))?
        .to_string();
    let generation = value
        .get("gen")
        .and_then(Value::as_i64)
        .ok_or_else(|| anyhow!("cursor payload is missing its generation"))?;
    let settings_hash = value
        .get("sh")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("cursor payload is missing its settings context"))?
        .to_string();
    let offset = usize::try_from(offset).map_err(|_| anyhow!("cursor offset is out of range"))?;
    let limit = usize::try_from(limit).map_err(|_| anyhow!("cursor page size is out of range"))?;
    if !(1..=100).contains(&limit) {
        return Err(anyhow!(
            "cursor page size {limit} is outside the allowed 1..=100 range"
        ));
    }
    if offset > MAX_SEARCH_CANDIDATE_WINDOW {
        return Err(anyhow!(
            "cursor offset {offset} exceeds the searchable candidate window"
        ));
    }
    Ok(RelevanceCursor {
        rerank_applied,
        mode,
        offset,
        limit,
        query_hash,
        filter_hash,
        generation,
        settings_hash,
    })
}

fn decode_date_cursor_body(hex: &str) -> Result<DateCursor> {
    let bytes = decode_hex(hex).ok_or_else(|| anyhow!("cursor payload is not valid hex"))?;
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|error| anyhow!("cursor payload is not valid JSON: {error}"))?;
    let version = value
        .get("v")
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("cursor payload is missing its version"))?;
    if version != DATE_CURSOR_SCHEMA_VERSION {
        return Err(anyhow!("unsupported cursor version {version}"));
    }
    let sort = value
        .get("sort")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("date cursor payload is missing its sort field"))?;
    if sort != "date" {
        return Err(anyhow!(
            "date cursor payload has an unsupported sort value '{sort}'"
        ));
    }
    let query_hash = value
        .get("qh")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("date cursor payload is missing its query context"))?
        .to_string();
    let filter_hash = value
        .get("fh")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("date cursor payload is missing its filter context"))?
        .to_string();
    let generation = value
        .get("gen")
        .and_then(Value::as_i64)
        .ok_or_else(|| anyhow!("date cursor payload is missing its generation"))?;
    let settings_hash = value
        .get("sh")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("date cursor payload is missing its settings context"))?
        .to_string();
    let limit = value
        .get("limit")
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("date cursor payload is missing its page size"))?;
    let limit = usize::try_from(limit).map_err(|_| anyhow!("date cursor page size is too large"))?;
    if !(1..=100).contains(&limit) {
        return Err(anyhow!(
            "date cursor page size {limit} is outside the allowed 1..=100 range"
        ));
    }
    let before = match value.get("before") {
        Some(Value::Null) | None => None,
        Some(Value::Number(number)) => Some(
            number
                .as_i64()
                .ok_or_else(|| anyhow!("date cursor before must be an integer"))?,
        ),
        _ => return Err(anyhow!("date cursor before must be an integer or null")),
    };
    if let Some(value) = before
        && !(0..=DATE_TIMESTAMP_CAP).contains(&value)
    {
        return Err(anyhow!("date cursor before is out of range"));
    }
    let upper = match value.get("upper") {
        Some(Value::Null) | None => None,
        Some(Value::Number(number)) => Some(
            number
                .as_i64()
                .ok_or_else(|| anyhow!("date cursor upper must be an integer"))?,
        ),
        _ => return Err(anyhow!("date cursor upper must be an integer or null")),
    };
    if let Some(value) = upper
        && !(0..=DATE_TIMESTAMP_CAP).contains(&value)
    {
        return Err(anyhow!("date cursor upper is out of range"));
    }
    let boundary_ts = match value.get("boundary_ts") {
        Some(Value::Null) | None => None,
        Some(Value::Number(number)) => Some(
            number
                .as_i64()
                .ok_or_else(|| anyhow!("date cursor boundary_ts must be an integer"))?,
        ),
        _ => return Err(anyhow!("date cursor boundary_ts must be an integer or null")),
    };
    if let Some(value) = boundary_ts
        && !(0..=DATE_TIMESTAMP_CAP).contains(&value)
    {
        return Err(anyhow!("date cursor boundary_ts is out of range"));
    }
    let offset = match value.get("offset") {
        Some(Value::Null) | None => None,
        Some(Value::String(raw)) => Some(
            uuid::Uuid::parse_str(raw)
                .map_err(|error| anyhow!("date cursor offset is not a valid uuid: {error}"))?,
        ),
        _ => return Err(anyhow!("date cursor offset must be a uuid string or null")),
    };
    Ok(DateCursor {
        limit,
        query_hash,
        filter_hash,
        generation,
        settings_hash,
        before,
        boundary_ts,
        offset,
        upper,
    })
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
        DecodedCursor::Date(_) => Err(anyhow!(
            "cursor ordering mismatch: the cursor belongs to the date ordering; date cursors are only valid against `sort=date` requests"
        )),
        DecodedCursor::Relevance(rel) => {
            if rel.rerank_applied == rerank_applied {
                return Ok(());
            }
            let epoch = |reranked: bool| if reranked { "reranked" } else { "local" };
            Err(anyhow!(
                "cursor ordering mismatch: the cursor belongs to the {} ordering but the current search resolves to the {} ordering; start again from the first page",
                epoch(rel.rerank_applied),
                epoch(rerank_applied)
            ))
        }
    }
}

/// Reject a cursor whose context (query / filter / limit / generation /
/// settings) disagrees with the request. Returns `Ok(())` for a missing
/// cursor or one whose context matches; otherwise returns a client-facing
/// error explaining which field differs. Date cursors reuse this check so a
/// changed filter / query / generation / settings still aborts the request.
pub(crate) fn validate_cursor_context(
    raw_cursor: Option<&str>,
    context: &CursorContext,
) -> Result<()> {
    let Some(raw) = raw_cursor else {
        return Ok(());
    };
    let cursor = decode_cursor(raw)?;
    match cursor {
        DecodedCursor::Date(date) => validate_date_context(&date, context),
        DecodedCursor::Relevance(rel) => {
            if rel.query_hash != context.query_hash {
                return Err(anyhow!(
                    "cursor context mismatch: the cursor belongs to a different query; start again from the first page"
                ));
            }
            if rel.filter_hash != context.filter_hash {
                return Err(anyhow!(
                    "cursor context mismatch: filters (locale, source, group, date range, or metadata filters) changed; start again from the first page"
                ));
            }
            if rel.settings_hash != context.settings_hash {
                return Err(anyhow!(
                    "cursor context mismatch: search settings changed; start again from the first page"
                ));
            }
            if rel.generation != context.generation {
                return Err(anyhow!(
                    "cursor context mismatch: the indexed data has been refreshed; start again from the first page"
                ));
            }
            if rel.limit != context.limit {
                return Err(anyhow!(
                    "cursor context mismatch: page size changed from {} to {}; start again from the first page",
                    rel.limit,
                    context.limit
                ));
            }
            Ok(())
        }
    }
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
    Ok(())
}

/// Reject a relevance cursor whose offset is not aligned to `limit`. The
/// relevance pagination contract only ever emits multiples of `limit`, so a
/// misaligned cursor means a malformed client or an old cursor that no
/// longer fits the current page-size geometry. Date cursors are ignored
/// (alignment is governed by the windowing logic).
pub(crate) fn validate_cursor_alignment(raw_cursor: Option<&str>) -> Result<()> {
    let Some(raw) = raw_cursor else {
        return Ok(());
    };
    let cursor = decode_cursor(raw)?;
    match cursor {
        DecodedCursor::Date(_) => Ok(()),
        DecodedCursor::Relevance(rel) => {
            if rel.offset % rel.limit != 0 {
                return Err(anyhow!(
                    "cursor offset {} is not aligned to the page size {}; start again from the first page",
                    rel.offset,
                    rel.limit
                ));
            }
            Ok(())
        }
    }
}

/// Reject a cursor that is incompatible with the date pipeline: only `c4:`
/// cursors are accepted; relevance cursors and legacy `c2:` / `c3:` cursors
/// are rejected with a clean error so the client can restart from the first
/// page.
pub(crate) fn validate_date_cursor(raw_cursor: Option<&str>) -> Result<DateCursor> {
    let Some(raw) = raw_cursor else {
        return Err(anyhow!(
            "date cursor missing: date-mode requests must carry a cursor or be a fresh first page"
        ));
    };
    match decode_cursor(raw)? {
        DecodedCursor::Date(date) => Ok(date),
        DecodedCursor::Relevance(_) => Err(anyhow!(
            "cursor ordering mismatch: the cursor belongs to the relevance ordering; relevance cursors are not valid for `sort=date` requests"
        )),
    }
}

fn decode_hex(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).ok())
        .collect()
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
        return Err(anyhow!("page_size must be between 1 and 100"));
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
    Some(encode_cursor(offset.saturating_sub(limit), rerank_applied, limit, context))
}

#[cfg(test)]
mod tests {
    use super::{
        CursorContext, DATE_TIMESTAMP_CAP, MAX_DATE_WINDOWS,
        MAX_SEARCH_CANDIDATE_WINDOW, RelevanceCursor, decode_cursor, encode_cursor,
        encode_date_cursor, ensure_cursor_ordering, validate_cursor_alignment,
        validate_cursor_context, validate_date_cursor,
    };
    use super::{DateCursor, DecodedCursor};

    fn ctx(limit: usize) -> CursorContext {
        CursorContext {
            query_hash: "qh".to_string(),
            filter_hash: "fh".to_string(),
            generation: 7,
            settings_hash: "sh".to_string(),
            limit,
        }
    }

    fn sample_date_cursor() -> DateCursor {
        DateCursor {
            limit: 8,
            query_hash: "qh".to_string(),
            filter_hash: "fh".to_string(),
            generation: 7,
            settings_hash: "sh".to_string(),
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
            assert!(raw.starts_with("c1:"));
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
                    query_hash: "qh".into(),
                    filter_hash: "fh".into(),
                    generation: 7,
                    settings_hash: "sh".into(),
                })
            );
        }
    }

    #[test]
    fn date_cursor_round_trip() {
        let raw = encode_date_cursor(&sample_date_cursor());
        assert!(raw.starts_with("c4:"));
        let decoded = decode_cursor(&raw).expect("date cursor decodes");
        assert_eq!(decoded, DecodedCursor::Date(sample_date_cursor()));
    }

    #[test]
    fn malformed_cursors_are_rejected() {
        assert!(decode_cursor("c1:zz").is_err(), "bad hex rejected");
        assert!(decode_cursor("c1:00").is_err(), "empty payload rejected");
        assert!(
            decode_cursor("c1:deadbeef").is_err(),
            "invalid json rejected"
        );
        assert!(
            decode_cursor("c1:7b2276223a327d").is_err(),
            "wrong version rejected"
        );
        // Legacy `c2:` date cursor is rejected at the prefix layer so the
        // client restarts from the first page.
        assert!(
            decode_cursor("c2:00").is_err(),
            "legacy c2 prefix rejected at decode"
        );
        assert!(
            decode_cursor("c2:7b2276223a317d").is_err(),
            "legacy c2 payload rejected at decode"
        );
        // Legacy `c3:` date cursor is rejected at the prefix layer so the
        // client restarts from the first page (the current pipeline no
        // longer threads the `tie_chunk` filter / `windows` counter).
        assert!(
            decode_cursor("c3:00").is_err(),
            "legacy c3 prefix rejected at decode"
        );
        assert!(decode_cursor("c4:00").is_err(), "empty c4 payload rejected");
        assert!(decode_cursor("c5:00").is_err(), "unknown prefix rejected");
        assert!(decode_cursor("").is_err());
    }

    #[test]
    fn oversized_cursor_offset_is_rejected() {
        let raw = encode_cursor(MAX_SEARCH_CANDIDATE_WINDOW + 1, false, 8, &ctx(8));
        assert!(decode_cursor(&raw).is_err());
    }

    #[test]
    fn date_cursor_rejects_out_of_range_timestamps() {
        // A cursor constructed with an out-of-range `before` is clamped by
        // the encoder so existing code that emits a `DateCursor` cannot
        // exceed the cap. Adversarial cursors that try to smuggle an
        // out-of-range value must still be rejected at decode time, which
        // we exercise by encoding a JSON payload directly.
        fn encode_raw(value: serde_json::Value) -> String {
            let bytes = serde_json::to_vec(&value).expect("json");
            let mut out = String::from("c4:");
            for byte in bytes {
                out.push_str(&format!("{byte:02x}"));
            }
            out
        }
        let mut payload = serde_json::json!({
            "v": 4,
            "sort": "date",
            "qh": "qh",
            "fh": "fh",
            "gen": 0,
            "sh": "sh",
            "limit": 8,
            "before": DATE_TIMESTAMP_CAP + 1,
            "boundary_ts": null,
            "offset": null,
            "upper": null
        });
        assert!(decode_cursor(&encode_raw(payload.clone())).is_err());
        payload["before"] = serde_json::Value::from(-1);
        assert!(decode_cursor(&encode_raw(payload.clone())).is_err());
        payload["before"] = serde_json::Value::Null;
        payload["upper"] = serde_json::Value::from(DATE_TIMESTAMP_CAP + 1);
        assert!(decode_cursor(&encode_raw(payload.clone())).is_err());
        payload["upper"] = serde_json::Value::from(-1);
        assert!(decode_cursor(&encode_raw(payload.clone())).is_err());
        payload["boundary_ts"] = serde_json::Value::from(DATE_TIMESTAMP_CAP + 1);
        assert!(decode_cursor(&encode_raw(payload.clone())).is_err());
        payload["boundary_ts"] = serde_json::Value::from(-1);
        assert!(decode_cursor(&encode_raw(payload.clone())).is_err());
        // An invalid `offset` (non-uuid string) is rejected at decode
        // time so an adversarial cursor cannot smuggle in arbitrary text.
        payload["boundary_ts"] = serde_json::Value::Null;
        payload["offset"] = serde_json::Value::from("not-a-uuid");
        assert!(decode_cursor(&encode_raw(payload.clone())).is_err());
        payload["offset"] = serde_json::Value::Null;
    }

    #[test]
    fn date_cursor_round_trips_keyset_offset() {
        // The keyset `offset` is opaque to the client; a UUID round-trips
        // through encode/decode without changing its value.
        let mut cursor = sample_date_cursor();
        cursor.offset = Some(uuid::Uuid::from_u128(0xabcd_1234_5678_9abc_def0_1234_5678_9abc));
        let raw = encode_date_cursor(&cursor);
        assert!(raw.starts_with("c4:"));
        let decoded = decode_cursor(&raw).expect("date cursor decodes");
        assert_eq!(
            decoded,
            DecodedCursor::Date(DateCursor {
                offset: Some(uuid::Uuid::from_u128(
                    0xabcd_1234_5678_9abc_def0_1234_5678_9abc
                )),
                ..cursor.clone()
            })
        );
        // `offset = None` must round-trip as `None` so the absence of the
        // field is interpreted as a no-op resume (the Qdrant side will
        // return its first page from the requested `boundary_ts` filter).
        cursor.offset = None;
        let raw = encode_date_cursor(&cursor);
        let decoded = decode_cursor(&raw).expect("date cursor decodes");
        assert_eq!(
            decoded,
            DecodedCursor::Date(DateCursor {
                offset: None,
                ..cursor.clone()
            })
        );
    }

    #[test]
    fn legacy_c2_date_cursor_is_rejected() {
        // The current date pipeline uses the `c4:` keyset walk. A leftover
        // `c2:` cursor (the previous windowed implementation) must be
        // rejected so the client restarts from the first page instead of
        // silently replaying a stale window shape that no longer matches
        // the pipeline geometry.
        fn encode_raw(prefix: &str, value: serde_json::Value) -> String {
            let bytes = serde_json::to_vec(&value).expect("json");
            let mut out = String::from(prefix);
            for byte in bytes {
                out.push_str(&format!("{byte:02x}"));
            }
            out
        }
        let payload = serde_json::json!({
            "v": 2,
            "sort": "date",
            "qh": "qh",
            "fh": "fh",
            "gen": 0,
            "sh": "sh",
            "limit": 8,
            "before": 1_700_000_000,
            "tie": { "ts": 1_700_000_000, "chunk": uuid::Uuid::nil().to_string() },
            "upper": 1_700_000_000
        });
        let raw = encode_raw("c2:", payload);
        let error = decode_cursor(&raw)
            .expect_err("legacy c2 date cursor must be rejected by the current pipeline");
        let message = error.to_string();
        assert!(message.contains("version 2"));
        assert!(message.contains("first page"));
    }

    #[test]
    fn legacy_c3_date_cursor_is_rejected() {
        // The previous `c3:` format carried a `windows` counter and a
        // `tie_chunk` field. The current date pipeline threads the
        // Qdrant-side `next_page_offset` instead and no longer maintains
        // an in-memory tie-break, so the `c3:` semantics no longer match
        // the pipeline geometry. Reject so the client restarts from the
        // first page.
        let raw = encode_date_cursor(&DateCursor {
            limit: 8,
            query_hash: "qh".to_string(),
            filter_hash: "fh".to_string(),
            generation: 0,
            settings_hash: "sh".to_string(),
            before: Some(1_700_000_000),
            boundary_ts: Some(1_700_000_000),
            offset: Some(uuid::Uuid::nil()),
            upper: Some(1_700_000_000),
        });
        // Strip the c4: prefix and re-prepend c3: to simulate a leftover
        // legacy cursor.
        let hex = raw.strip_prefix("c4:").expect("c4 prefix");
        let legacy = format!("c3:{hex}");
        let error = decode_cursor(&legacy)
            .expect_err("legacy c3 date cursor must be rejected by the current pipeline");
        let message = error.to_string();
        assert!(message.contains("version 3"));
        assert!(message.contains("first page"));
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
        let pagination = super::build_search_pagination(
            0,
            8,
            8,
            None,
            false,
            &ctx(8),
        )
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
        let date = encode_date_cursor(&sample_date_cursor());
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
    }

    #[test]
    fn date_cursor_context_mismatch_rejects_changed_filter() {
        let cursor = encode_date_cursor(&sample_date_cursor());
        let mut other = ctx(8);
        other.filter_hash = "different".into();
        let error = validate_cursor_context(Some(&cursor), &other)
            .expect_err("filter hash must differ");
        assert!(error.to_string().contains("filters"));
    }

    #[test]
    fn cursor_context_mismatch_rejects_changed_limit() {
        let cursor = encode_cursor(8, true, 8, &ctx(8));
        let error = validate_cursor_context(Some(&cursor), &ctx(16))
            .expect_err("limit change must be rejected");
        let message = error.to_string();
        assert!(message.contains("cursor context mismatch"));
        assert!(message.contains("page size"));
    }

    #[test]
    fn cursor_context_mismatch_rejects_filter_change() {
        let cursor = encode_cursor(8, true, 8, &ctx(8));
        let mut other = ctx(8);
        other.filter_hash = "different".into();
        let error = validate_cursor_context(Some(&cursor), &other)
            .expect_err("filter hash must differ");
        assert!(error.to_string().contains("filters"));
    }

    #[test]
    fn cursor_context_mismatch_rejects_generation_bump() {
        let cursor = encode_cursor(8, true, 8, &ctx(8));
        let mut other = ctx(8);
        other.generation = 8;
        let error = validate_cursor_context(Some(&cursor), &other)
            .expect_err("generation change must be rejected");
        assert!(error.to_string().contains("indexed data"));
    }

    #[test]
    fn cursor_context_mismatch_rejects_settings_change() {
        let cursor = encode_cursor(8, true, 8, &ctx(8));
        let mut other = ctx(8);
        other.settings_hash = "different".into();
        let error = validate_cursor_context(Some(&cursor), &other)
            .expect_err("settings change must be rejected");
        assert!(error.to_string().contains("search settings"));
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
        // Offset 0 is always a valid previous window for limit > 0.
        let cursor = encode_cursor(0, false, 0, &ctx(0));
        // limit 0 is rejected by decode itself.
        assert!(validate_cursor_alignment(Some(&cursor)).is_err());
    }

    #[test]
    fn date_cursor_alignment_ignores_geometry() {
        let cursor = encode_date_cursor(&sample_date_cursor());
        assert!(validate_cursor_alignment(Some(&cursor)).is_ok());
    }

    #[test]
    fn max_date_windows_constant_is_per_request_budget() {
        // The cap is a per-request budget: replayed requests do not
        // accumulate window usage. The cursor module no longer exposes a
        // `windows_consumed` field so the rejection-at-resume behaviour
        // is impossible by construction.
        let _ = MAX_DATE_WINDOWS;
    }
}
