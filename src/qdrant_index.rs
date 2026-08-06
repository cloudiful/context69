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
        let started = Instant::now();
        timeout(
            QDRANT_OPERATION_TIMEOUT,
            self.client
                .upsert_points(UpsertPointsBuilder::new(&self.collection_name, points).wait(true)),
        )
        .await
        .map_err(|_| {
            anyhow!(
                "qdrant points upsert request timed out after {}s",
                QDRANT_OPERATION_TIMEOUT.as_secs()
            )
        })?
        .context("qdrant points upsert request failed")?;
        info!(
            batch_size,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "qdrant points upserted"
        );
        Ok(())
    }

    pub async fn update_chunk_payloads(&self, payloads: &[ChunkPayload]) -> Result<()> {
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

        timeout(
            QDRANT_OPERATION_TIMEOUT,
            self.client.update_points_batch(
                UpdateBatchPointsBuilder::new(&self.collection_name, operations).wait(true),
            ),
        )
        .await
        .map_err(|_| {
            anyhow!(
                "qdrant points update request timed out after {}s",
                QDRANT_OPERATION_TIMEOUT.as_secs()
            )
        })?
        .context("qdrant points update request failed")?;
        info!(
            batch_size = payloads.len(),
            elapsed_ms = started.elapsed().as_millis() as u64,
            "qdrant payload batch updated"
        );
        Ok(())
    }

    pub async fn delete_points(&self, chunk_ids: &[Uuid]) -> Result<()> {
        if chunk_ids.is_empty() {
            return Ok(());
        }

        timeout(
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
        .map_err(|_| {
            anyhow!(
                "qdrant points delete request timed out after {}s",
                QDRANT_OPERATION_TIMEOUT.as_secs()
            )
        })?
        .context("qdrant points delete request failed")?;
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
        let result = self
            .client
            .search_points(builder)
            .await
            .context("qdrant search request failed")?;
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
        let result = self
            .client
            .count(CountPointsBuilder::new(&self.collection_name).exact(true))
            .await
            .context("qdrant count request failed")?;
        Ok(result.result.map(|count| count.count).unwrap_or_default())
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
