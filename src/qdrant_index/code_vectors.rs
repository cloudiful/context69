//! Isolated Qdrant destination and payload for Git code vectors.
//!
//! Code vectors live in their own versioned collection, never in the document
//! collection and never under the document identity. A point id is the canonical
//! stored chunk key, and the payload carries only the provenance P3 needs to
//! hydrate a hit: group, repository, generation, file, chunk index, path,
//! language, and line range.

use anyhow::{Result, anyhow};
use qdrant_client::{
    Payload,
    qdrant::{Condition, Filter, GetPointsBuilder, PointId, PointStruct},
};
use serde_json::json;
use uuid::Uuid;

use super::{QDRANT_OPERATION_TIMEOUT, QdrantIndex, SearchPointHit, point_id_to_uuid};

/// Prefix of the physical collection one code destination owns.
pub(crate) const CODE_COLLECTION_PREFIX: &str = "context69_code_v1_";

/// The collection name of one code destination fingerprint.
pub(crate) fn code_collection_name(destination_fingerprint: &str) -> String {
    format!("{CODE_COLLECTION_PREFIX}{destination_fingerprint}")
}

/// One code vector point before it becomes a Qdrant `PointStruct`.
#[derive(Debug, Clone)]
pub(crate) struct CodeVectorPoint {
    pub(crate) chunk_key: Uuid,
    pub(crate) vector: Vec<f32>,
    pub(crate) group_id: i64,
    pub(crate) repository_key: Uuid,
    pub(crate) generation_key: Uuid,
    pub(crate) file_key: Uuid,
    pub(crate) chunk_index: i32,
    pub(crate) path: String,
    pub(crate) language: String,
    pub(crate) start_line: i32,
    pub(crate) end_line: i32,
}

/// The provenance payload of one code point.
pub(crate) fn code_vector_payload(point: &CodeVectorPoint) -> serde_json::Value {
    json!({
        "chunk_key": point.chunk_key.to_string(),
        "group_id": point.group_id,
        "repository_key": point.repository_key.to_string(),
        "generation_key": point.generation_key.to_string(),
        "file_key": point.file_key.to_string(),
        "chunk_index": point.chunk_index,
        "path": point.path,
        "language": point.language,
        "start_line": point.start_line,
        "end_line": point.end_line,
    })
}

/// Builds the Qdrant points, rejecting a wrong dimension or a non-finite value
/// before any request is issued.
pub(crate) fn build_code_points(
    points: &[CodeVectorPoint],
    dimensions: usize,
) -> Result<Vec<PointStruct>> {
    let mut built = Vec::with_capacity(points.len());
    for (index, point) in points.iter().enumerate() {
        if point.vector.len() != dimensions {
            return Err(anyhow!(
                "code embedding dimension mismatch at point {index}: expected {dimensions}, got {}",
                point.vector.len()
            ));
        }
        if !point.vector.iter().all(|value| value.is_finite()) {
            return Err(anyhow!(
                "code embedding contains a non-finite value at point {index}"
            ));
        }
        built.push(PointStruct::new(
            point.chunk_key.to_string(),
            point.vector.clone(),
            Payload::try_from(code_vector_payload(point))?,
        ));
    }
    Ok(built)
}

impl QdrantIndex {
    /// Read-only semantic search inside one code generation's collection.
    ///
    /// The caller must have attached to the recorded collection with
    /// `attach_code_read_only_collection` first; this method never creates, ensures,
    /// or bootstraps a collection. Filters are exact group/repository/generation
    /// matches, and hits are deduplicated by chunk key while keeping the highest
    /// score order Qdrant returns.
    pub(crate) async fn search_code_vectors(
        &self,
        group_id: i64,
        repository_key: Uuid,
        generation_key: Uuid,
        vector: &[f32],
        limit: i64,
    ) -> Result<Vec<SearchPointHit>> {
        if vector.len() != self.dimensions {
            return Err(anyhow!(
                "code query dimension mismatch: expected {}, got {}",
                self.dimensions,
                vector.len()
            ));
        }
        if !vector.iter().all(|value| value.is_finite()) {
            return Err(anyhow!("code query vector contains a non-finite value"));
        }
        if limit <= 0 {
            return Ok(Vec::new());
        }
        let filter = Filter::must([
            Condition::matches("group_id", group_id),
            Condition::matches("repository_key", repository_key.to_string()),
            Condition::matches("generation_key", generation_key.to_string()),
        ]);
        let hits = self
            .search_point_ids("search_code_points", vector.to_vec(), limit as u64, filter)
            .await?;
        let mut deduped: Vec<SearchPointHit> = Vec::new();
        for hit in hits {
            if deduped
                .iter()
                .any(|existing| existing.chunk_id == hit.chunk_id)
            {
                continue;
            }
            deduped.push(hit);
        }
        Ok(deduped)
    }

    /// Writes one bounded page of code points insert-only with `wait=true`.
    ///
    /// An existing point id keeps its stored payload and vector, so a retried
    /// generation never overwrites an already acknowledged code point.
    pub(crate) async fn upsert_code_points(&self, points: &[CodeVectorPoint]) -> Result<()> {
        if points.is_empty() {
            return Ok(());
        }
        let built = build_code_points(points, self.dimensions)?;
        self.upsert_points_insert_only(built, points.len()).await
    }

    /// Reads the stored vectors of the requested chunk ids, skipping ids the
    /// collection no longer holds.
    ///
    /// Only the exact point ids are requested, so a copy pass never scans or
    /// filters the collection. A vector whose dimension does not match this
    /// destination is omitted rather than copied into the new generation.
    pub(crate) async fn retrieve_code_vectors(
        &self,
        point_ids: &[Uuid],
    ) -> Result<Vec<(Uuid, Vec<f32>)>> {
        if self.is_noop() || point_ids.is_empty() {
            return Ok(Vec::new());
        }
        let collection = self.collection_name.clone();
        let extra = format!("point_count={}", point_ids.len());
        let request = GetPointsBuilder::new(
            &self.collection_name,
            point_ids
                .iter()
                .map(|id| PointId::from(id.to_string()))
                .collect::<Vec<_>>(),
        )
        .with_vectors(true)
        .build();
        let response =
            tokio::time::timeout(QDRANT_OPERATION_TIMEOUT, self.client.get_points(request))
                .await
                .map_err(|_| anyhow!("qdrant get_points timed out for {collection} ({extra})"))?
                .map_err(|error| {
                    anyhow!("qdrant get_points failed for {collection} ({extra}): {error}")
                })?;

        let mut vectors = Vec::new();
        for point in response.result {
            let Some(point_id) = point.id else {
                continue;
            };
            let chunk_key = point_id_to_uuid(point_id)?;
            let Some(vectors_output) = point.vectors else {
                continue;
            };
            let Some(qdrant_client::qdrant::vectors_output::VectorsOptions::Vector(output)) =
                vectors_output.vectors_options
            else {
                continue;
            };
            let Some(qdrant_client::qdrant::vector_output::Vector::Dense(dense)) = output.vector
            else {
                continue;
            };
            if dense.data.len() == self.dimensions {
                vectors.push((chunk_key, dense.data));
            }
        }
        Ok(vectors)
    }
}
