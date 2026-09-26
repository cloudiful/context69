use anyhow::{Context, Result};
use qdrant_client::qdrant::{Condition, Filter, PointId, Range};
use std::time::Instant;
use tracing::info;

use super::errors::format_qdrant_error;
use super::search::i64_to_f64;
use super::{QdrantIndex, point_id_to_uuid};
use crate::contracts::SearchRequest;
use crate::domain::AccessScope;

impl QdrantIndex {
    /// Latest-first date-mode window fetch using Qdrant's `scroll` API
    /// with a `published_ts` order. The shared filter builder produces the
    /// same access / locale / source / metadata conditions as the
    /// relevance search; only the `published_ts` constraint, the
    /// order-by field, and the keyset offset are date-specific.
    ///
    /// The function returns at most `limit` points whose `published_ts`
    /// value satisfies the inclusive `before` bound and the exclusive
    /// `after` bound (both `Option<i64>`). The order is
    /// `published_ts DESC`; the Qdrant `scroll` return order IS the
    /// authoritative ordering for the date-mode pipeline. The Qdrant
    /// `OrderBy` API only supports a single key (qdrant-client 1.19),
    /// so the within-boundary order is whatever the Qdrant `scroll`
    /// emits — the pipeline does NOT re-sort in memory after
    /// hydration. Same-second ties are therefore stable per ordering
    /// epoch but not `chunk_id ASC` (the legacy in-memory tie-break
    /// was retired when `tie_chunk` was removed; see
    /// `search_cursor::DateCursor` + the date-mode header docs).
    ///
    /// `offset` is the Qdrant-side keyset cursor for paging within the
    /// same `published_ts` value. `None` means "start from the first
    /// point in the window". The returned `next_offset` is `Some(uuid)`
    /// when the Qdrant side knows more points satisfy the filter so the
    /// pipeline can keep paging without losing records; the date
    /// pipeline threads this resume key into the per-timestamp drain
    /// AND surfaces it in the opaque `DateCursor` so a replayed
    /// request resumes the boundary in the same scroll order. The
    /// pipeline additionally bounds the per-timestamp drain with a
    /// hard cap to keep the walk finite.
    pub async fn search_by_date_window(
        &self,
        _vector: Vec<f32>,
        request: &SearchRequest,
        scope: &AccessScope,
        query: context69_search::DateWindowQuery,
    ) -> Result<crate::services::query::DateWindowPage> {
        use crate::services::query::DateWindowPage;
        use context69_search::DateBound;
        let after_exclusive = match query.after {
            DateBound::Inclusive(value) => Some(value.saturating_sub(1)),
            DateBound::Unbounded => None,
        };
        let before_inclusive = match query.before {
            DateBound::Inclusive(value) => Some(value),
            DateBound::Unbounded => None,
        };
        let limit = query.limit;
        let offset = query.offset;
        if limit == 0 {
            return Ok(DateWindowPage {
                hits: Vec::new(),
                next_offset: None,
            });
        }
        let conditions = self.build_search_conditions(request, scope)?;
        if conditions.is_short_circuit {
            return Ok(DateWindowPage {
                hits: Vec::new(),
                next_offset: None,
            });
        }
        let range = Range {
            gt: after_exclusive.map(i64_to_f64),
            lte: before_inclusive.map(i64_to_f64),
            ..Default::default()
        };
        let mut filter = conditions.filter;
        filter.push(Condition::range("published_ts", range));

        let mut builder = qdrant_client::qdrant::ScrollPointsBuilder::new(&self.collection_name)
            .filter(Filter::must(filter))
            .order_by(date_window_order_by())
            .limit(limit as u32)
            .with_payload(
                qdrant_client::qdrant::with_payload_selector::SelectorOptions::Enable(true),
            );
        if let Some(offset_uuid) = offset.as_ref() {
            builder = builder.offset(PointId::from(offset_uuid.to_string()));
        }

        let started = Instant::now();
        let operation = "search_by_date_window";
        let collection = self.collection_name.clone();
        let extra = format!("limit={limit}");
        let response = self
            .client
            .scroll(builder)
            .await
            .map_err(|err| format_qdrant_error(operation, &collection, &extra, err.into()))?;
        let hits: Vec<crate::services::query::SearchDatePointHit> = response
            .result
            .into_iter()
            .map(|point| {
                let point_id = point.id.context("missing qdrant point id")?;
                let chunk_id = point_id_to_uuid(point_id)?;
                let published_ts = match point
                    .order_value
                    .as_ref()
                    .and_then(|value| value.variant.as_ref())
                {
                    Some(qdrant_client::qdrant::order_value::Variant::Int(value)) => Some(*value),
                    _ => point
                        .payload
                        .get("published_ts")
                        .and_then(|value| match value.kind {
                            Some(qdrant_client::qdrant::value::Kind::IntegerValue(integer)) => {
                                Some(integer)
                            }
                            _ => None,
                        }),
                };
                Ok(crate::services::query::SearchDatePointHit {
                    chunk_id,
                    published_ts,
                    score: 0.0,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let next_offset: Option<uuid::Uuid> =
            response
                .next_page_offset
                .as_ref()
                .and_then(|pid| match &pid.point_id_options {
                    Some(qdrant_client::qdrant::point_id::PointIdOptions::Uuid(value)) => {
                        uuid::Uuid::parse_str(value).ok()
                    }
                    _ => None,
                });
        info!(
            candidate_count = hits.len(),
            has_next_offset = next_offset.is_some(),
            metadata_filter_count = request.metadata_filters.len(),
            elapsed_ms = started.elapsed().as_millis() as u64,
            "qdrant date window completed"
        );
        Ok(DateWindowPage { hits, next_offset })
    }

    /// Snapshot the maximum `published_ts` value visible under the request's
    /// filter set. The query plan issues one `scroll` ordered by
    /// `published_ts DESC` with limit 1 so the result is the latest record
    /// the user can see under their current filters. When the index has no
    /// visible rows, returns `Ok(None)`.
    pub async fn date_max_published_ts(
        &self,
        request: &SearchRequest,
        scope: &AccessScope,
    ) -> Result<Option<i64>> {
        let conditions = self.build_search_conditions(request, scope)?;
        if conditions.is_short_circuit {
            return Ok(None);
        }
        let builder = qdrant_client::qdrant::ScrollPointsBuilder::new(&self.collection_name)
            .filter(Filter::must(conditions.filter))
            .order_by(date_window_order_by())
            .limit(1)
            .with_payload(
                qdrant_client::qdrant::with_payload_selector::SelectorOptions::Enable(true),
            );
        let operation = "date_max_published_ts";
        let collection = self.collection_name.clone();
        let extra = "limit=1".to_string();
        let response = self
            .client
            .scroll(builder)
            .await
            .map_err(|err| format_qdrant_error(operation, &collection, &extra, err.into()))?;
        Ok(response.result.into_iter().find_map(|point| {
            point
                .order_value
                .as_ref()
                .and_then(|value| match value.variant.as_ref() {
                    Some(qdrant_client::qdrant::order_value::Variant::Int(value)) => Some(*value),
                    _ => None,
                })
                .or_else(|| {
                    point
                        .payload
                        .get("published_ts")
                        .and_then(|value| match value.kind {
                            Some(qdrant_client::qdrant::value::Kind::IntegerValue(integer)) => {
                                Some(integer)
                            }
                            _ => None,
                        })
                })
        }))
    }
}

/// Build the Qdrant `OrderBy` used by the date-mode window walk. The order
/// is a non-vector key — by `published_ts` descending — so the returned
/// ordering is dictated by the payload field, not the similarity score. The
/// `score` field of the response is therefore only kept for observability;
/// the date pipeline keeps the Qdrant `scroll` return order verbatim and
/// threads the Qdrant-side `next_page_offset` through the per-timestamp
/// drain + the opaque `DateCursor` (see `search_by_date_window` and
/// `search_cursor::DateCursor` for the full resume contract). The legacy
/// `chunk_id ASC` tie-break is NOT applied here: the qdrant-client 1.19
/// `OrderBy` API only supports a single key, so the within-boundary order
/// is whatever the Qdrant `scroll` emits and is stable per ordering epoch.
fn date_window_order_by() -> qdrant_client::qdrant::OrderBy {
    use qdrant_client::qdrant::{Direction, OrderByBuilder};

    OrderByBuilder::new("published_ts".to_string())
        .direction(Direction::Desc as i32)
        .build()
}
