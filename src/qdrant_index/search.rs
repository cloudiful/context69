use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use qdrant_client::qdrant::{Condition, CountPointsBuilder, Filter, Range, SearchPointsBuilder};
use std::time::Instant;
use tracing::info;

use super::errors::format_qdrant_error;
use super::payload::metadata_filter_condition;
use super::{QdrantIndex, SearchPointHit, date_to_timestamp, point_id_to_uuid};
use crate::contracts::SearchRequest;
use crate::domain::AccessScope;

impl QdrantIndex {
    pub async fn search(
        &self,
        vector: Vec<f32>,
        request: &SearchRequest,
        scope: &AccessScope,
    ) -> Result<Vec<SearchPointHit>> {
        let conditions = self.build_search_conditions(request, scope)?;
        if conditions.is_short_circuit {
            return Ok(Vec::new());
        }

        let builder = if conditions.filter.is_empty() {
            SearchPointsBuilder::new(&self.collection_name, vector, request.limit as u64)
        } else {
            SearchPointsBuilder::new(&self.collection_name, vector, request.limit as u64)
                .filter(Filter::must(conditions.filter))
        };

        let started = Instant::now();
        let operation = "search_points";
        let collection = self.collection_name.clone();
        let extra = format!("limit={}", request.limit);
        let result = self
            .client
            .search_points(builder)
            .await
            .map_err(|err| format_qdrant_error(operation, &collection, &extra, err.into()))?;
        let hits = result
            .result
            .into_iter()
            .map(|point| {
                let point_id = point.id.context("missing qdrant point id")?;
                let chunk_id = point_id_to_uuid(point_id)?;
                Ok(SearchPointHit {
                    chunk_id,
                    score: point.score,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        info!(
            candidate_count = hits.len(),
            metadata_filter_count = request.metadata_filters.len(),
            elapsed_ms = started.elapsed().as_millis() as u64,
            "qdrant search completed"
        );
        Ok(hits)
    }

    /// Build the access/locale/source/date/metadata conditions shared by the
    /// relevance search and the date-mode window. Returns
    /// `(filter_conditions, is_short_circuit)` where `is_short_circuit = true`
    /// means the request resolves to an empty result set (e.g. a scoped group
    /// path that no longer maps to a group id) and the caller should skip the
    /// Qdrant round-trip.
    pub(super) fn build_search_conditions(
        &self,
        request: &SearchRequest,
        scope: &AccessScope,
    ) -> Result<BuiltSearchConditions> {
        let mut filter = Vec::new();

        if let Some(source_key) = &request.source_key {
            filter.push(Condition::matches("source_key", source_key.clone()));
        }

        let locale_filter = match request.locale.as_deref() {
            Some(locale) => Filter::should(vec![
                Condition::matches("content_locale", locale.to_string()),
                Condition::matches("content_locale", "original".to_string()),
                Condition::is_empty("content_locale"),
            ]),
            None => Filter::should(vec![
                Condition::matches("content_locale", "original".to_string()),
                Condition::is_empty("content_locale"),
            ]),
        };
        filter.push(Condition::from(locale_filter));

        if request.published_after.is_some() || request.published_before.is_some() {
            let range = Range {
                gte: request.published_after.map(date_to_timestamp_f64),
                lte: request.published_before.map(date_to_timestamp_f64),
                ..Default::default()
            };
            filter.push(Condition::range("published_ts", range));
        }

        filter.extend(
            request
                .metadata_filters
                .iter()
                .filter_map(metadata_filter_condition),
        );

        if let Some(group_id) = scope.scoped_group_id {
            filter.push(Condition::matches("group_id", group_id));
        } else if scope.group_path.is_some() {
            // The request was scoped to a group path that no longer resolves.
            // Returning nothing is safer than silently widening the scope.
            return Ok(BuiltSearchConditions {
                filter,
                is_short_circuit: true,
            });
        }

        let access_condition = if scope.private_group_ids.is_empty() {
            Condition::matches("visibility", "public".to_string())
        } else {
            Condition::from(Filter::should(vec![
                Condition::matches("visibility", "public".to_string()),
                Condition::matches("group_id", scope.private_group_ids.clone()),
            ]))
        };
        filter.push(access_condition);

        Ok(BuiltSearchConditions {
            filter,
            is_short_circuit: false,
        })
    }

    pub async fn count_points(&self) -> Result<u64> {
        let operation = "count_points";
        let collection = self.collection_name.clone();
        let extra = String::new();
        let result = self
            .client
            .count(CountPointsBuilder::new(&self.collection_name).exact(true))
            .await
            .map_err(|err| format_qdrant_error(operation, &collection, &extra, err.into()))?;
        Ok(result.result.map(|count| count.count).unwrap_or_default())
    }
}

/// Output of `QdrantIndex::build_search_conditions`. `is_short_circuit = true`
/// means the conditions (e.g. a scoped-but-missing group) resolve to an empty
/// result set and the caller should skip the Qdrant round-trip.
pub(super) struct BuiltSearchConditions {
    pub(super) filter: Vec<Condition>,
    pub(super) is_short_circuit: bool,
}

fn date_to_timestamp_f64(date: DateTime<Utc>) -> f64 {
    i64_to_f64(date_to_timestamp(date))
}

pub(super) fn i64_to_f64(value: i64) -> f64 {
    value as f64
}
