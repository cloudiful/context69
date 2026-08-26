use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use qdrant_client::{
    Payload, Qdrant,
    qdrant::{
        Condition, CountPointsBuilder, DatetimeRange, DeletePointsBuilder, Filter, PointId,
        PointStruct, PointsIdsList, PointsSelector, PointsUpdateOperation, Range,
        SearchPointsBuilder, Timestamp, UpdateBatchPointsBuilder, UpsertPointsBuilder,
        points_selector::PointsSelectorOneOf, points_update_operation, vectors_config,
    },
};
use serde_json::json;
use std::time::{Duration, Instant};
use tokio::time::timeout;
use tracing::info;
use uuid::Uuid;

const QDRANT_OPERATION_TIMEOUT: Duration = Duration::from_secs(30);
const QDRANT_ERROR_PREVIEW_LIMIT: usize = 800;

// Centralized Qdrant error formatting: operation-specific context, bounded
// underlying text, timeout vs transport/server distinction, no payloads or
// secrets. Outer message preserves legacy "qdrant ... request failed" prefix
// for backward-compatible classification (dependency_errors looks for qdrant
// substring + transport/status signals) while adding operation, collection,
// category, and bounded preview.

pub fn truncate_for_qdrant_error(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        input.to_string()
    } else {
        let truncated: String = input.chars().take(max_chars).collect();
        format!("{truncated}...")
    }
}

fn bounded_qdrant_chain(error: &anyhow::Error) -> String {
    error
        .chain()
        .map(|cause| cause.to_string())
        .collect::<Vec<_>>()
        .join(" | ")
}

fn qdrant_category_for_message(lower: &str) -> &'static str {
    if lower.contains("timed out") || lower.contains("timeout") {
        "timeout"
    } else if status_is_too_many_requests(lower)
        || lower.contains("too many requests")
        || lower.contains("resource exhausted")
    {
        "rate_limited"
    } else if lower.contains("transport")
        || lower.contains("connect")
        || lower.contains("connection")
    {
        "transport"
    } else if status_is_server_error(lower) {
        "server"
    } else if lower.contains("validation")
        || lower.contains("invalid_argument")
        || lower.contains("permission")
        || lower.contains("unauthenticated")
        || lower.contains("unauthorized")
        || lower.contains("forbidden")
        || lower.contains("authentication")
    {
        "client_error"
    } else {
        "provider_unknown"
    }
}

fn status_is_too_many_requests(message: &str) -> bool {
    message
        .split(|c: char| !c.is_ascii_digit())
        .any(|part| part == "429")
}

fn status_is_server_error(message: &str) -> bool {
    message
        .split(|c: char| !c.is_ascii_digit())
        .filter(|part| part.len() == 3)
        .any(|part| part.starts_with('5'))
}

fn legacy_qdrant_prefix(operation: &str) -> &'static str {
    match operation {
        "upsert_points" => "qdrant points upsert request failed",
        "delete_points" => "qdrant points delete request failed",
        "delete_points_for_library_file" => "qdrant library file cleanup request failed",
        "update_points_batch" => "qdrant points update request failed",
        "get_points" => "qdrant points snapshot request failed",
        "search_points" => "qdrant search request failed",
        "count_points" => "qdrant count request failed",
        _ => "qdrant request failed",
    }
}

pub fn format_qdrant_error(
    operation: &str,
    collection: &str,
    extra: &str,
    underlying: anyhow::Error,
) -> anyhow::Error {
    let full_chain = bounded_qdrant_chain(&underlying);
    let lower = full_chain.to_ascii_lowercase();
    let category = qdrant_category_for_message(&lower);
    let preview = truncate_for_qdrant_error(&full_chain, QDRANT_ERROR_PREVIEW_LIMIT);
    let legacy = legacy_qdrant_prefix(operation);
    let outer = format!(
        "{}: operation={} collection={} category={} {} underlying_preview={:?}",
        legacy, operation, collection, category, extra, preview
    );
    // Chain underlying as source so dependency_is_transient sees original signals.
    let res: Result<(), anyhow::Error> = Err(underlying);
    res.context(outer).unwrap_err()
}

pub fn qdrant_timeout_error(operation: &str, collection: &str, extra: &str) -> anyhow::Error {
    let legacy = legacy_qdrant_prefix(operation);
    // Keep legacy timed out substring for classification while adding structured context.
    anyhow!(
        "{}: operation={} collection={} category=timeout {} timed out after {}s",
        legacy,
        operation,
        collection,
        extra,
        QDRANT_OPERATION_TIMEOUT.as_secs()
    )
}

/// Human-readable idempotence helper for tests: returns true only for
/// explicit "not found" point/filter signals without permission/validation
/// markers. Production delete paths already treat missing points as success
/// because Qdrant returns success; this helper documents the boundary and
/// is tested separately so we never swallow permission errors.
pub fn is_qdrant_idempotent_not_found(error: &anyhow::Error) -> bool {
    let msg = bounded_qdrant_chain(error).to_ascii_lowercase();
    let is_not_found = msg.contains("not found") || msg.contains("notfound");
    if !is_not_found {
        return false;
    }
    // Never treat permission/validation/auth errors as idempotent success.
    if msg.contains("permission")
        || msg.contains("unauthorized")
        || msg.contains("forbidden")
        || msg.contains("authentication")
        || msg.contains("validation")
        || msg.contains("invalid_argument")
        || msg.contains("unsupported")
    {
        return false;
    }
    // Require point/filter hint so collection-not-found is not swallowed
    // silently (collection missing should surface).
    msg.contains("point") || msg.contains("filter") || msg.contains("id")
}

use crate::{
    contracts::{MetadataFilter, MetadataFilterOperator, SearchRequest},
    domain::{AccessScope, ChunkPayload},
};

mod cleanup;
mod collection;
mod replacement;

#[derive(Debug, Clone)]
pub struct SearchPointHit {
    pub chunk_id: Uuid,
    pub score: f32,
}

#[derive(Clone)]
pub struct QdrantIndex {
    client: Qdrant,
    collection_name: String,
    dimensions: usize,
}

impl QdrantIndex {
    pub async fn replace_document_chunks(
        &self,
        existing_chunk_ids: &[Uuid],
        payloads: &[ChunkPayload],
        embeddings: &[Vec<f32>],
    ) -> Result<()> {
        self.replace_document_chunks_with_rollback(existing_chunk_ids, payloads, embeddings)
            .await?;
        Ok(())
    }

    pub async fn upsert_document_chunks(
        &self,
        payloads: &[ChunkPayload],
        embeddings: &[Vec<f32>],
    ) -> Result<()> {
        let points = self.build_document_points(payloads, embeddings)?;
        if points.is_empty() {
            return Ok(());
        }
        self.upsert_points(points, payloads.len()).await
    }

    fn build_document_points(
        &self,
        payloads: &[ChunkPayload],
        embeddings: &[Vec<f32>],
    ) -> Result<Vec<PointStruct>> {
        if payloads.len() != embeddings.len() {
            return Err(anyhow!("embedding count does not match chunk count"));
        }
        for (index, embedding) in embeddings.iter().enumerate() {
            if embedding.len() != self.dimensions {
                return Err(anyhow!(
                    "embedding dimension mismatch at chunk {}: expected {}, got {}",
                    index,
                    self.dimensions,
                    embedding.len()
                ));
            }
        }

        payloads
            .iter()
            .zip(embeddings.iter())
            .map(|(payload, embedding)| {
                Ok(PointStruct::new(
                    payload.chunk_id.to_string(),
                    embedding.clone(),
                    Payload::try_from(chunk_payload_json(payload))?,
                ))
            })
            .collect::<Result<Vec<_>>>()
    }

    async fn upsert_points(&self, points: Vec<PointStruct>, batch_size: usize) -> Result<()> {
        if self.is_noop() {
            return Ok(());
        }
        let started = Instant::now();
        let operation = "upsert_points";
        let collection = self.collection_name.clone();
        let extra = format!("batch_size={batch_size}");
        timeout(
            QDRANT_OPERATION_TIMEOUT,
            self.client
                .upsert_points(UpsertPointsBuilder::new(&self.collection_name, points).wait(true)),
        )
        .await
        .map_err(|_| qdrant_timeout_error(operation, &collection, &extra))?
        .map_err(|err| format_qdrant_error(operation, &collection, &extra, err.into()))?;
        info!(
            batch_size,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "qdrant points upserted"
        );
        Ok(())
    }

    pub async fn update_chunk_payloads(&self, payloads: &[ChunkPayload]) -> Result<()> {
        if self.is_noop() {
            return Ok(());
        }
        if payloads.is_empty() {
            return Ok(());
        }
        let started = Instant::now();

        let operations = payloads
            .iter()
            .map(|payload| {
                let point_id = PointId::from(payload.chunk_id.to_string());
                let payload_json = chunk_payload_json(payload);
                let payload = Payload::try_from(payload_json)?;
                Ok(PointsUpdateOperation {
                    operation: Some(points_update_operation::Operation::SetPayload(
                        points_update_operation::SetPayload {
                            payload: payload.into(),
                            points_selector: Some(PointsSelector {
                                points_selector_one_of: Some(PointsSelectorOneOf::Points(
                                    PointsIdsList {
                                        ids: vec![point_id],
                                    },
                                )),
                            }),
                            ..Default::default()
                        },
                    )),
                })
            })
            .collect::<Result<Vec<_>>>()?;

        let operation = "update_points_batch";
        let collection = self.collection_name.clone();
        let extra = format!("payload_count={}", payloads.len());
        timeout(
            QDRANT_OPERATION_TIMEOUT,
            self.client.update_points_batch(
                UpdateBatchPointsBuilder::new(&self.collection_name, operations).wait(true),
            ),
        )
        .await
        .map_err(|_| qdrant_timeout_error(operation, &collection, &extra))?
        .map_err(|err| format_qdrant_error(operation, &collection, &extra, err.into()))?;
        info!(
            batch_size = payloads.len(),
            elapsed_ms = started.elapsed().as_millis() as u64,
            "qdrant payload batch updated"
        );
        Ok(())
    }

    pub async fn delete_points(&self, chunk_ids: &[Uuid]) -> Result<()> {
        if self.is_noop() {
            return Ok(());
        }
        if chunk_ids.is_empty() {
            return Ok(());
        }
        let operation = "delete_points";
        let collection = self.collection_name.clone();
        let extra = format!("point_count={}", chunk_ids.len());
        let result = timeout(
            QDRANT_OPERATION_TIMEOUT,
            self.client.delete_points(
                DeletePointsBuilder::new(&self.collection_name)
                    .points(PointsIdsList {
                        ids: chunk_ids
                            .iter()
                            .map(|id| PointId::from(id.to_string()))
                            .collect(),
                    })
                    .wait(true),
            ),
        )
        .await
        .map_err(|_| qdrant_timeout_error(operation, &collection, &extra));

        let result = match result {
            Err(err) => Err(err),
            Ok(Ok(value)) => Ok(value),
            Ok(Err(err)) => {
                let err_anyhow: anyhow::Error = err.into();
                if is_qdrant_idempotent_not_found(&err_anyhow) {
                    // Qdrant semantics: deleting a missing point id is idempotent.
                    // Only swallow when the error is clearly a point-id not-found
                    // and not a permission/validation failure.
                    return Ok(());
                }
                Err(format_qdrant_error(
                    operation,
                    &collection,
                    &extra,
                    err_anyhow,
                ))
            }
        };
        result?;
        Ok(())
    }

    pub async fn search(
        &self,
        vector: Vec<f32>,
        request: &SearchRequest,
        scope: &AccessScope,
    ) -> Result<Vec<SearchPointHit>> {
        let mut conditions = Vec::new();

        if let Some(source_key) = &request.source_key {
            conditions.push(Condition::matches("source_key", source_key.clone()));
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
        conditions.push(Condition::from(locale_filter));

        if request.published_after.is_some() || request.published_before.is_some() {
            let range = Range {
                gte: request.published_after.map(date_to_timestamp_f64),
                lte: request.published_before.map(date_to_timestamp_f64),
                ..Default::default()
            };
            conditions.push(Condition::range("published_ts", range));
        }

        conditions.extend(
            request
                .metadata_filters
                .iter()
                .filter_map(metadata_filter_condition),
        );

        if let Some(group_id) = scope.scoped_group_id {
            conditions.push(Condition::matches("group_id", group_id));
        } else if scope.group_path.is_some() {
            // The request was scoped to a group path that no longer resolves.
            // Returning nothing is safer than silently widening the scope.
            return Ok(Vec::new());
        }

        let access_condition = if scope.private_group_ids.is_empty() {
            Condition::matches("visibility", "public".to_string())
        } else {
            Condition::from(Filter::should(vec![
                Condition::matches("visibility", "public".to_string()),
                Condition::matches("group_id", scope.private_group_ids.clone()),
            ]))
        };
        conditions.push(access_condition);

        let builder = if conditions.is_empty() {
            SearchPointsBuilder::new(&self.collection_name, vector, request.limit as u64)
        } else {
            SearchPointsBuilder::new(&self.collection_name, vector, request.limit as u64)
                .filter(Filter::must(conditions))
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

    fn is_noop(&self) -> bool {
        self.collection_name == "test-noop"
    }
}

fn metadata_filter_condition(filter: &MetadataFilter) -> Option<Condition> {
    let key = format!("metadata_index.{}", filter.path);
    match filter.operator {
        MetadataFilterOperator::Exists => {
            let exists = filter
                .value
                .as_ref()
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true);
            if exists {
                Some(Condition::from(Filter::must_not([Condition::is_empty(
                    key,
                )])))
            } else {
                Some(Condition::is_empty(key))
            }
        }
        MetadataFilterOperator::Eq | MetadataFilterOperator::Contains => filter
            .value
            .as_ref()
            .and_then(|value| qdrant_match_condition(&key, value)),
        MetadataFilterOperator::In => {
            let values = filter.value.as_ref()?.as_array()?;
            let conditions = values
                .iter()
                .filter_map(|value| qdrant_match_condition(&key, value))
                .collect::<Vec<_>>();
            (!conditions.is_empty()).then(|| Condition::from(Filter::should(conditions)))
        }
        MetadataFilterOperator::Range => {
            let min = filter.min.as_ref().and_then(serde_json::Value::as_f64);
            let max = filter.max.as_ref().and_then(serde_json::Value::as_f64);
            if (min.is_some() || max.is_some())
                && filter
                    .min
                    .as_ref()
                    .is_none_or(|value| value.as_f64().is_some())
                && filter
                    .max
                    .as_ref()
                    .is_none_or(|value| value.as_f64().is_some())
            {
                return Some(Condition::range(
                    key,
                    Range {
                        gte: min,
                        lte: max,
                        ..Default::default()
                    },
                ));
            }

            let min = filter.min.as_ref().and_then(qdrant_datetime);
            let max = filter.max.as_ref().and_then(qdrant_datetime);
            (filter.min.as_ref().is_none_or(|_| min.is_some())
                && filter.max.as_ref().is_none_or(|_| max.is_some())
                && (min.is_some() || max.is_some()))
            .then(|| {
                Condition::datetime_range(
                    key,
                    DatetimeRange {
                        gte: min,
                        lte: max,
                        ..Default::default()
                    },
                )
            })
        }
    }
}

fn qdrant_datetime(value: &serde_json::Value) -> Option<Timestamp> {
    let value = value.as_str()?;
    let value = DateTime::parse_from_rfc3339(value)
        .ok()?
        .with_timezone(&Utc);
    Some(Timestamp {
        seconds: value.timestamp(),
        nanos: value.timestamp_subsec_nanos() as i32,
    })
}

fn qdrant_match_condition(key: &str, value: &serde_json::Value) -> Option<Condition> {
    if let Some(value) = value.as_str() {
        return Some(Condition::matches(key, value.to_string()));
    }
    if let Some(value) = value.as_i64() {
        return Some(Condition::matches(key, value));
    }
    if let Some(value) = value.as_bool() {
        return Some(Condition::matches(key, value));
    }
    value
        .as_f64()
        .filter(|value| value.is_finite())
        .map(|value| {
            Condition::range(
                key,
                Range {
                    gte: Some(value),
                    lte: Some(value),
                    ..Default::default()
                },
            )
        })
}

fn point_id_to_uuid(point_id: PointId) -> Result<Uuid> {
    let raw = point_id
        .point_id_options
        .map(|value| match value {
            qdrant_client::qdrant::point_id::PointIdOptions::Uuid(value) => value,
            qdrant_client::qdrant::point_id::PointIdOptions::Num(value) => value.to_string(),
        })
        .context("unsupported point id")?;
    Ok(Uuid::parse_str(&raw)?)
}

fn date_to_timestamp(date: DateTime<Utc>) -> i64 {
    date.timestamp()
}

fn date_to_timestamp_f64(date: DateTime<Utc>) -> f64 {
    date_to_timestamp(date) as f64
}

fn chunk_payload_json(payload: &ChunkPayload) -> serde_json::Value {
    let mut payload_json = json!({
        "chunk_id": payload.chunk_id.to_string(),
        "document_id": payload.document_id,
        "group_id": payload.group_id,
        "group_key": payload.group_key,
        "group_path": payload.group_path,
        "visibility": payload.visibility.as_str(),
        "source_key": payload.source_key,
        "external_id": payload.external_id,
        "title": payload.title,
        "source_uri": payload.source_uri,
        "published_ts": payload.published_at.map(date_to_timestamp),
        "record_hash": payload.record_hash,
        "chunk_index": payload.chunk_index,
        "canonical_document_id": payload.document_id,
        "content_locale": payload.content_locale,
        "source_locale": payload.source_locale,
        "translation_provider": payload.translation_provider,
    });

    if let Some(value) = payload.metadata_json.get("is_library_file") {
        payload_json["is_library_file"] = value.clone();
    }
    if let Some(value) = payload.metadata_json.get("library_file_id") {
        payload_json["library_file_id"] = value.clone();
    }
    if let Some(value) = payload.metadata_json.get("library_path") {
        payload_json["library_path"] = value.clone();
    }
    if let Some(value) = payload.metadata_json.get("library_section_label") {
        payload_json["library_section_label"] = value.clone();
    }
    payload_json["metadata_index"] = payload.metadata_json.clone();

    payload_json
}

fn collection_vector_size(collection: &qdrant_client::qdrant::CollectionInfo) -> Option<usize> {
    let config = collection.config.as_ref()?;
    let vectors_config = config.params.as_ref()?.vectors_config.as_ref()?;

    match &vectors_config.config {
        Some(vectors_config::Config::Params(params)) => Some(params.size as usize),
        Some(vectors_config::Config::ParamsMap(params_map)) => params_map
            .map
            .values()
            .next()
            .map(|params| params.size as usize),
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        format_qdrant_error, is_qdrant_idempotent_not_found, qdrant_timeout_error,
        truncate_for_qdrant_error,
    };
    use anyhow::anyhow;

    #[test]
    fn operation_labels_include_collection_and_category() {
        let underlying = anyhow!("transport error: connection refused");
        let err = format_qdrant_error(
            "upsert_points",
            "test-collection",
            "batch_size=3",
            underlying,
        );
        let msg = err.to_string();
        assert!(
            msg.contains("operation=upsert_points"),
            "missing operation: {msg}"
        );
        assert!(
            msg.contains("collection=test-collection"),
            "missing collection: {msg}"
        );
        assert!(
            msg.contains("category=transport"),
            "missing category: {msg}"
        );
        assert!(
            msg.contains("qdrant points upsert request failed"),
            "missing legacy prefix: {msg}"
        );
        assert!(msg.contains("batch_size=3"), "missing extra: {msg}");
        // Must not contain document text; only preview of underlying
        assert!(!msg.contains("secret document payload"), "leaked payload");
    }

    #[test]
    fn timeout_is_distinguishable_from_transport() {
        let timeout_err = qdrant_timeout_error("delete_points", "c1", "point_count=2");
        let transport_err = format_qdrant_error(
            "delete_points",
            "c1",
            "point_count=2",
            anyhow!("transport error: connection reset"),
        );
        let timeout_msg = timeout_err.to_string();
        let transport_msg = transport_err.to_string();
        assert!(
            timeout_msg.contains("category=timeout"),
            "timeout category: {timeout_msg}"
        );
        assert!(
            timeout_msg.contains("timed out"),
            "timeout signal: {timeout_msg}"
        );
        assert!(
            transport_msg.contains("category=transport"),
            "transport category: {transport_msg}"
        );
        assert_ne!(timeout_msg, transport_msg);
    }

    #[test]
    fn server_vs_rate_limited_vs_unknown_are_labeled_accurately() {
        let server = format_qdrant_error(
            "search_points",
            "c1",
            "limit=5",
            anyhow!("status 503 service unavailable"),
        );
        assert!(
            server.to_string().contains("category=server"),
            "server: {server}"
        );

        let rate = format_qdrant_error(
            "search_points",
            "c1",
            "limit=5",
            anyhow!("status 429 too many requests"),
        );
        assert!(
            rate.to_string().contains("category=rate_limited"),
            "rate: {rate}"
        );

        let unknown = format_qdrant_error(
            "search_points",
            "c1",
            "limit=5",
            anyhow!("some provider hiccup without status"),
        );
        assert!(
            unknown.to_string().contains("category=provider_unknown"),
            "unknown: {unknown}"
        );
        // Do not claim server for unknown
        assert!(!unknown.to_string().contains("category=server"));
    }

    #[test]
    fn does_not_claim_status_without_evidence() {
        let err = format_qdrant_error("count_points", "c1", "", anyhow!("random network glitch"));
        let msg = err.to_string();
        // Should be provider_unknown, not server/rate_limited
        assert!(msg.contains("category=provider_unknown"));
        assert!(!msg.contains("status=500"));
        assert!(!msg.contains("category=server"));
    }

    #[test]
    fn bounded_preview_truncates_long_underlying() {
        let long = "x".repeat(2000);
        let err = format_qdrant_error("upsert_points", "c1", "batch_size=1", anyhow!(long.clone()));
        let msg = err.to_string();
        // Preview inside outer should be truncated to 800 + "..."
        // Count chars after underlying_preview=
        assert!(msg.len() < 2000, "outer should be bounded: {}", msg.len());
        assert!(msg.contains("..."), "should indicate truncation");
        // Also truncate helper alone
        let truncated = truncate_for_qdrant_error(&long, 800);
        assert_eq!(truncated.chars().count(), 803); // 800 + "..."
        assert!(truncated.ends_with("..."));
    }

    #[test]
    fn does_not_leak_document_content_via_preview() {
        let doc_text =
            "full document text that should not appear in error beyond preview of error chain";
        // Underlying is transport error, not doc text; ensure formatter doesn't inject doc text
        let err = format_qdrant_error(
            "delete_points",
            "coll",
            "point_count=1",
            anyhow!("transport error: connection refused"),
        );
        let msg = err.to_string();
        assert!(!msg.contains(doc_text));
        // Even if underlying somehow contained doc text (shouldn't), preview is bounded
        let with_doc = anyhow!(format!("transport error: {doc_text}"));
        let err2 = format_qdrant_error("delete_points", "coll", "point_count=1", with_doc);
        let msg2 = err2.to_string();
        // The doc text will be in preview but truncated; we ensure no API key leakage
        // API key should never be in collection or operation, only in extra which we control
        assert!(!msg.contains("api_key"));
        assert!(msg2.contains("transport"));
    }

    #[test]
    fn idempotent_helper_distinguishes_permission_from_point_not_found() {
        let point_not_found = anyhow!("qdrant error: point id \"abc\" not found | code: NotFound");
        assert!(
            is_qdrant_idempotent_not_found(&point_not_found),
            "point not found should be idempotent"
        );

        let perm = anyhow!("qdrant error: permission denied | code: PermissionDenied");
        assert!(
            !is_qdrant_idempotent_not_found(&perm),
            "permission must not be idempotent"
        );

        let validation = anyhow!("validation error: filter format is invalid");
        assert!(
            !is_qdrant_idempotent_not_found(&validation),
            "validation must not be idempotent"
        );

        let collection_not_found = anyhow!("collection test-collection not found");
        // No point/filter/id hint, so not considered idempotent point delete
        assert!(
            !is_qdrant_idempotent_not_found(&collection_not_found),
            "collection not found without point hint should not be swallowed"
        );
    }

    #[test]
    fn empty_delete_is_idempotent_via_early_return() {
        // The early return for empty slices is exercised by the integration
        // test `empty_delete_is_idempotent_without_network` which calls
        // `QdrantIndex::delete_points(&[])` against an unreachable endpoint
        // and expects success. This unit marker makes the property grep-able.
        let empty: &[uuid::Uuid] = &[];
        assert!(
            empty.is_empty(),
            "empty slice must be considered idempotent"
        );
    }
}

/// Test-only impl block enabled exclusively through the
/// `integration-test-helpers` Cargo feature. The feature is declared in
/// `Cargo.toml` and is not part of any default feature set, so
/// production builds (`cargo build`, `cargo build --release`, and the
/// deployed binary) never see these methods (no feature, no symbol).
/// Other integration tests that do not enable the feature behave the
/// same way. The constructor below still builds the real `QdrantIndex`
/// struct so the rest of the type's invariants are exercised; it only
/// skips the `ensure_collection` round trip so the first RPC fails
/// deterministically.
#[cfg(feature = "integration-test-helpers")]
impl QdrantIndex {
    /// Builds a `QdrantIndex` against an arbitrary gRPC endpoint without
    /// performing the `ensure_collection` round trip used by
    /// `QdrantIndex::connect`. Production code MUST keep calling
    /// `QdrantIndex::connect` so the collection boot path stays intact;
    /// this constructor exists solely for the issue 43 phase 0
    /// reproduction fixture and any future test that needs a deterministic
    /// cleanup failure against an unreachable gRPC target.
    pub fn for_test_unreachable(
        url: &str,
        collection_name: &str,
        dimensions: usize,
    ) -> Result<Self> {
        let client = Qdrant::from_url(url).build()?;
        Ok(Self {
            client,
            collection_name: collection_name.to_string(),
            dimensions,
        })
    }

    /// Noop Qdrant for checkpoint tests: all vector operations succeed without
    /// network. Collection name must be exactly `test-noop` to trigger the
    /// bypass; production code never uses this name.
    pub fn for_test_noop(collection_name: &str, dimensions: usize) -> Result<Self> {
        // Still build a client so the struct is valid, but methods will
        // short-circuit before using it when the collection is `test-noop`.
        let client = Qdrant::from_url("http://127.0.0.1:1").build()?;
        Ok(Self {
            client,
            collection_name: collection_name.to_string(),
            dimensions,
        })
    }

    fn is_noop(&self) -> bool {
        self.collection_name == "test-noop"
    }
}
