use anyhow::{Context, Result, anyhow};
use chrono::NaiveDate;
use qdrant_client::{
    Payload, Qdrant,
    qdrant::{
        Condition, CountPointsBuilder, CreateCollectionBuilder, CreateFieldIndexCollectionBuilder,
        DeletePointsBuilder, Distance, FieldType, Filter, PointId, PointStruct, PointsIdsList,
        Range, SearchPointsBuilder, SetPayloadPointsBuilder, UpsertPointsBuilder,
        VectorParamsBuilder, vectors_config,
    },
};
use serde_json::json;
use tracing::warn;
use uuid::Uuid;

use crate::{config::QdrantConfig, contracts::SearchRequest, domain::{AccessScope, ChunkPayload}};

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
    pub async fn connect(config: &QdrantConfig, dimensions: usize) -> Result<(Self, bool)> {
        let client = Qdrant::from_url(&config.url).build()?;
        let index = Self {
            client,
            collection_name: config.collection_name.clone(),
            dimensions,
        };
        let recreated = index
            .ensure_collection(dimensions, config.recreate_on_dimension_mismatch)
            .await
            .with_context(|| {
                format!(
                    "failed to initialize qdrant collection '{}' at {}; qdrant-client uses the gRPC endpoint, usually port 6334, not the REST endpoint on 6333",
                    config.collection_name, config.url
                )
            })?;
        Ok((index, recreated))
    }

    async fn ensure_collection(
        &self,
        dimensions: usize,
        recreate_on_dimension_mismatch: bool,
    ) -> Result<bool> {
        if self.client.collection_exists(&self.collection_name).await? {
            let collection = self
                .client
                .collection_info(&self.collection_name)
                .await?
                .result
                .context("missing qdrant collection info")?;
            let actual_dimensions = collection_vector_size(&collection)
                .context("missing qdrant vector size in collection config")?;
            if actual_dimensions != dimensions {
                if recreate_on_dimension_mismatch {
                    warn!(
                        collection_name = self.collection_name,
                        expected_dimensions = dimensions,
                        actual_dimensions,
                        "qdrant collection dimension mismatch detected, recreating collection"
                    );
                    self.client.delete_collection(&self.collection_name).await?;
                    self.create_collection(dimensions).await?;
                    return Ok(true);
                }
                return Err(anyhow!(
                    "qdrant collection {} dimension mismatch: expected {}, found {}",
                    self.collection_name,
                    dimensions,
                    actual_dimensions
                ));
            }
            return Ok(false);
        }

        self.create_collection(dimensions).await?;
        Ok(false)
    }

    async fn create_collection(&self, dimensions: usize) -> Result<()> {
        self.client
            .create_collection(
                CreateCollectionBuilder::new(&self.collection_name).vectors_config(
                    VectorParamsBuilder::new(dimensions as u64, Distance::Cosine),
                ),
            )
            .await?;
        self.client
            .create_field_index(
                CreateFieldIndexCollectionBuilder::new(
                    self.collection_name.clone(),
                    "group_key",
                    FieldType::Keyword,
                )
                .wait(true),
            )
            .await
            .ok();
        self.client
            .create_field_index(
                CreateFieldIndexCollectionBuilder::new(
                    self.collection_name.clone(),
                    "project_key",
                    FieldType::Keyword,
                )
                .wait(true),
            )
            .await
            .ok();
        self.client
            .create_field_index(
                CreateFieldIndexCollectionBuilder::new(
                    self.collection_name.clone(),
                    "project_id",
                    FieldType::Integer,
                )
                .wait(true),
            )
            .await
            .ok();
        self.client
            .create_field_index(
                CreateFieldIndexCollectionBuilder::new(
                    self.collection_name.clone(),
                    "visibility",
                    FieldType::Keyword,
                )
                .wait(true),
            )
            .await
            .ok();
        self.client
            .create_field_index(
                CreateFieldIndexCollectionBuilder::new(
                    self.collection_name.clone(),
                    "source_key",
                    FieldType::Keyword,
                )
                .wait(true),
            )
            .await
            .ok();
        self.client
            .create_field_index(
                CreateFieldIndexCollectionBuilder::new(
                    self.collection_name.clone(),
                    "document_id",
                    FieldType::Integer,
                )
                .wait(true),
            )
            .await
            .ok();
        self.client
            .create_field_index(
                CreateFieldIndexCollectionBuilder::new(
                    self.collection_name.clone(),
                    "published_ts",
                    FieldType::Integer,
                )
                .wait(true),
            )
            .await
            .ok();
        Ok(())
    }

    pub async fn replace_document_chunks(
        &self,
        existing_chunk_ids: &[Uuid],
        payloads: &[ChunkPayload],
        embeddings: &[Vec<f32>],
    ) -> Result<()> {
        self.delete_points(existing_chunk_ids).await?;

        if payloads.is_empty() {
            return Ok(());
        }

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

        let points = payloads
            .iter()
            .zip(embeddings.iter())
            .map(|(payload, embedding)| {
                Ok(PointStruct::new(
                    payload.chunk_id.to_string(),
                    embedding.clone(),
                    Payload::try_from(chunk_payload_json(payload))?,
                ))
            })
            .collect::<Result<Vec<_>>>()?;

        self.client
            .upsert_points(UpsertPointsBuilder::new(&self.collection_name, points).wait(true))
            .await?;
        Ok(())
    }

    pub async fn update_chunk_payloads(&self, payloads: &[ChunkPayload]) -> Result<()> {
        for payload in payloads {
            let payload_json = Payload::try_from(chunk_payload_json(payload))?;
            self.client
                .set_payload(
                    SetPayloadPointsBuilder::new(&self.collection_name, payload_json)
                        .points_selector(PointsIdsList {
                            ids: vec![PointId::from(payload.chunk_id.to_string())],
                        })
                        .wait(true),
                )
                .await?;
        }
        Ok(())
    }

    pub async fn delete_points(&self, chunk_ids: &[Uuid]) -> Result<()> {
        if chunk_ids.is_empty() {
            return Ok(());
        }

        self.client
            .delete_points(
                DeletePointsBuilder::new(&self.collection_name)
                    .points(PointsIdsList {
                        ids: chunk_ids
                            .iter()
                            .map(|id| PointId::from(id.to_string()))
                            .collect(),
                    })
                    .wait(true),
            )
            .await?;
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

        if request.published_after.is_some() || request.published_before.is_some() {
            let range = Range {
                gte: request.published_after.map(date_to_timestamp_f64),
                lte: request.published_before.map(date_to_timestamp_f64),
                ..Default::default()
            };
            conditions.push(Condition::range("published_ts", range));
        }

        if let Some(group_key) = &scope.group_key {
            conditions.push(Condition::matches("group_key", group_key.clone()));
        }

        if let Some(project_key) = &scope.project_key {
            conditions.push(Condition::matches("project_key", project_key.clone()));
        }

        let access_condition = if scope.private_project_ids.is_empty() {
            Condition::matches("visibility", "public".to_string())
        } else {
            Condition::from(Filter::should(vec![
                Condition::matches("visibility", "public".to_string()),
                Condition::matches("project_id", scope.private_project_ids.clone()),
            ]))
        };
        conditions.push(access_condition);

        let builder = if conditions.is_empty() {
            SearchPointsBuilder::new(&self.collection_name, vector, request.limit as u64)
        } else {
            SearchPointsBuilder::new(&self.collection_name, vector, request.limit as u64)
                .filter(Filter::must(conditions))
        };

        let result = self.client.search_points(builder).await?;
        result
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
            .collect()
    }

    pub async fn count_points(&self) -> Result<u64> {
        let result = self
            .client
            .count(CountPointsBuilder::new(&self.collection_name).exact(true))
            .await?;
        Ok(result.result.map(|count| count.count).unwrap_or_default())
    }
}

fn point_id_to_uuid(point_id: PointId) -> Result<Uuid> {
    let raw = point_id
        .point_id_options
        .and_then(|value| match value {
            qdrant_client::qdrant::point_id::PointIdOptions::Uuid(value) => Some(value),
            qdrant_client::qdrant::point_id::PointIdOptions::Num(value) => Some(value.to_string()),
        })
        .context("unsupported point id")?;
    Ok(Uuid::parse_str(&raw)?)
}

fn date_to_timestamp(date: NaiveDate) -> i64 {
    date.and_hms_opt(0, 0, 0)
        .map(|value| value.and_utc().timestamp())
        .unwrap_or_default()
}

fn date_to_timestamp_f64(date: NaiveDate) -> f64 {
    date_to_timestamp(date) as f64
}

fn chunk_payload_json(payload: &ChunkPayload) -> serde_json::Value {
    let mut payload_json = json!({
        "chunk_id": payload.chunk_id.to_string(),
        "document_id": payload.document_id,
        "group_id": payload.group_id,
        "group_key": payload.group_key,
        "project_id": payload.project_id,
        "project_key": payload.project_key,
        "visibility": payload.visibility.as_str(),
        "source_key": payload.source_key,
        "external_id": payload.external_id,
        "title": payload.title,
        "source_uri": payload.source_uri,
        "published_ts": payload.published_at.map(date_to_timestamp),
        "record_hash": payload.record_hash,
        "chunk_index": payload.chunk_index,
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
