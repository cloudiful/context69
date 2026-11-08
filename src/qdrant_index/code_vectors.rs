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
    qdrant::{Condition, Filter, PointStruct, SearchPointsBuilder},
};
use serde_json::json;
use uuid::Uuid;

use super::errors::format_qdrant_error;
use super::{QdrantIndex, point_id_to_uuid};

/// Prefix of the physical collection one code destination owns.
pub(crate) const CODE_COLLECTION_PREFIX: &str = "context69_code_v1_";

/// One read-only semantic code hit: the canonical chunk key plus Qdrant cosine
/// score. Text and provenance come only from scoped database hydration.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CodeVectorQueryHit {
    pub(crate) chunk_key: Uuid,
    pub(crate) score: f32,
}

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
    /// `attach_read_only_collection` first; this method never creates, ensures,
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
    ) -> Result<Vec<CodeVectorQueryHit>> {
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
        let builder =
            SearchPointsBuilder::new(&self.collection_name, vector.to_vec(), limit as u64)
                .filter(filter);
        let result = self.client.search_points(builder).await.map_err(|error| {
            format_qdrant_error(
                "search_code_points",
                &self.collection_name,
                &format!("limit={limit}"),
                error.into(),
            )
        })?;
        let mut hits: Vec<CodeVectorQueryHit> = Vec::new();
        for point in result.result {
            let Some(point_id) = point.id else {
                continue;
            };
            let chunk_key = point_id_to_uuid(point_id)?;
            if hits.iter().any(|hit| hit.chunk_key == chunk_key) {
                continue;
            }
            hits.push(CodeVectorQueryHit {
                chunk_key,
                score: point.score,
            });
        }
        Ok(hits)
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
}

#[cfg(test)]
mod live_tests {
    use std::env;

    use anyhow::{Context, Result, ensure};
    use uuid::Uuid;

    use super::{CODE_COLLECTION_PREFIX, CodeVectorPoint, QdrantIndex};
    use crate::config::QdrantConfig;

    const EXPECTED_HOST: &str = "unraid.cloud1ful.com";
    const EXPECTED_REST_PORT: u16 = 6335;
    const TEST_GRPC_PORT: u16 = 6336;
    const PRODUCTION_GRPC_PORT: u16 = 6334;
    const AUTHORIZED_REST_URL: &str = "http://unraid.cloud1ful.com:6335";
    const URL_ENV: &str = "CONTEXT69_TEST_QDRANT_URL";
    const EXPECT_ENV: &str = "CONTEXT69_TEST_QDRANT_EXPECT";
    const DIMENSIONS: usize = 8;

    /// Exercises the production ensure → upsert → read-only search path against
    /// the single authorized test endpoint.
    ///
    /// The collection is a unique task-owned scratch name, inspected absent
    /// before creation and deleted afterwards. The document collection is never
    /// named, attached, or modified.
    #[tokio::test]
    #[ignore = "requires CONTEXT69_TEST_QDRANT_URL and CONTEXT69_TEST_QDRANT_EXPECT for the authorized test endpoint"]
    async fn production_code_vectors_round_trip_and_filter() -> Result<()> {
        let url = env::var(URL_ENV).context("missing test Qdrant URL")?;
        let expect = env::var(EXPECT_ENV).context("missing expected Qdrant authority")?;
        // `qdrant-client` speaks gRPC only, so the production path needs the
        // test instance's gRPC companion; the authorized REST endpoint proves
        // which instance this is, and the production gRPC port is refused.
        ensure!(
            url == format!("http://{EXPECTED_HOST}:{TEST_GRPC_PORT}"),
            "the production gRPC test URL must be the authorized test instance's gRPC companion"
        );
        ensure!(
            expect == format!("{EXPECTED_HOST}:{EXPECTED_REST_PORT}"),
            "the expected authority must be the authorized test REST authority"
        );
        ensure!(
            !url.contains('@') && !url.to_ascii_lowercase().contains("password"),
            "the test endpoint must not carry credentials"
        );
        ensure!(
            TEST_GRPC_PORT != PRODUCTION_GRPC_PORT,
            "the production gRPC port must never be the test target"
        );
        ensure!(
            AUTHORIZED_REST_URL == format!("http://{EXPECTED_HOST}:{EXPECTED_REST_PORT}"),
            "the authorized REST endpoint must match the expected test authority"
        );

        let collection = format!("{CODE_COLLECTION_PREFIX}ci_{}", Uuid::new_v4());
        ensure!(
            collection.starts_with(CODE_COLLECTION_PREFIX),
            "the scratch name follows the production code-collection prefix"
        );
        println!("scratch code collection (task-owned, created by this run): {collection}");

        let config = QdrantConfig {
            url: url.clone(),
            collection_name: collection.clone(),
            recreate_on_dimension_mismatch: false,
        };
        ensure!(
            QdrantIndex::inspect_collection(&config, &collection, DIMENSIONS)
                .await?
                .is_none(),
            "refusing to touch a collection this run did not create"
        );

        let group_id = 7001;
        let repository_key = Uuid::new_v4();
        let generation_key = Uuid::new_v4();
        let other_key = Uuid::new_v4();
        let chunk_a = Uuid::new_v4();
        let chunk_b = Uuid::new_v4();

        let index = QdrantIndex::ensure_code_collection(&config, &collection, DIMENSIONS).await?;
        let points = vec![
            point(group_id, repository_key, generation_key, chunk_a, 1.0, 0.0),
            point(group_id, repository_key, generation_key, chunk_b, 0.0, 1.0),
        ];
        index.upsert_code_points(&points).await?;
        println!("acknowledged {} synthetic code points", points.len());

        let reader =
            QdrantIndex::attach_read_only_collection(&config, &collection, DIMENSIONS).await?;
        let matching = reader
            .search_code_vectors(
                group_id,
                repository_key,
                generation_key,
                &[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
                10,
            )
            .await?;
        ensure!(
            matching.iter().any(|hit| hit.chunk_key == chunk_a),
            "the production search must find the matching code point"
        );
        for (label, found) in [
            (
                "foreign group",
                reader
                    .search_code_vectors(
                        group_id + 1,
                        repository_key,
                        generation_key,
                        &[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
                        10,
                    )
                    .await?,
            ),
            (
                "foreign repository",
                reader
                    .search_code_vectors(
                        group_id,
                        other_key,
                        generation_key,
                        &[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
                        10,
                    )
                    .await?,
            ),
            (
                "foreign generation",
                reader
                    .search_code_vectors(
                        group_id,
                        repository_key,
                        other_key,
                        &[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
                        10,
                    )
                    .await?,
            ),
        ] {
            ensure!(found.is_empty(), "{label} must be excluded by the filter");
        }

        index
            .client
            .delete_collection(&collection)
            .await
            .context("delete the task-owned scratch collection")?;
        ensure!(
            QdrantIndex::inspect_collection(&config, &collection, DIMENSIONS)
                .await?
                .is_none(),
            "the task-owned scratch collection must be deleted"
        );
        println!("deleted task-owned scratch collection {collection}");
        Ok(())
    }

    fn point(
        group_id: i64,
        repository_key: Uuid,
        generation_key: Uuid,
        chunk_key: Uuid,
        first: f32,
        second: f32,
    ) -> CodeVectorPoint {
        CodeVectorPoint {
            chunk_key,
            vector: vec![first, second, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            group_id,
            repository_key,
            generation_key,
            file_key: Uuid::new_v4(),
            chunk_index: 0,
            path: "src/lib.rs".to_string(),
            language: "rust".to_string(),
            start_line: 1,
            end_line: 2,
        }
    }
}
