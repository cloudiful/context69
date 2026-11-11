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

#[cfg(test)]
mod live_tests {
    use std::env;

    use anyhow::{Context, Result, ensure};
    use qdrant_client::{
        Qdrant,
        qdrant::{CreateCollectionBuilder, Distance, VectorParamsBuilder},
    };
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

    fn authorized_test_endpoint() -> Result<String> {
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
        Ok(url)
    }

    /// Exercises the production ensure → upsert → read-only search path against
    /// the single authorized test endpoint.
    ///
    /// The collection is a unique task-owned scratch name, inspected absent
    /// before creation and deleted afterwards. The document collection is never
    /// named, attached, or modified.
    #[tokio::test]
    #[ignore = "requires CONTEXT69_TEST_QDRANT_URL and CONTEXT69_TEST_QDRANT_EXPECT for the authorized test endpoint"]
    async fn production_code_vectors_round_trip_and_filter() -> Result<()> {
        let url = authorized_test_endpoint()?;

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
            QdrantIndex::attach_code_read_only_collection(&config, &collection, DIMENSIONS).await?;
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
            matching.iter().any(|hit| hit.chunk_id == chunk_a),
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

    /// Creates unique task-owned incompatible collections (a non-Cosine single
    /// vector and a Cosine multi-vector), then proves the code ensure and
    /// read-only attach both refuse each without mutating, deleting, or
    /// recreating it. Every collection is inspected absent before creation and
    /// deleted only after its refusal checks.
    #[tokio::test]
    #[ignore = "requires CONTEXT69_TEST_QDRANT_URL and CONTEXT69_TEST_QDRANT_EXPECT for the authorized test endpoint"]
    async fn production_code_collection_rejects_a_non_cosine_distance() -> Result<()> {
        let url = authorized_test_endpoint()?;
        let client = Qdrant::from_url(&url).build()?;
        let cases = [
            ("dot", Distance::Dot, None, "non-Cosine single-vector"),
            (
                "maxsim",
                Distance::Cosine,
                Some(qdrant_client::qdrant::MultiVectorConfig::default()),
                "Cosine multi-vector",
            ),
        ];

        for (suffix, distance, multivector, label) in cases {
            let collection = format!("{CODE_COLLECTION_PREFIX}{suffix}_{}", Uuid::new_v4());
            println!(
                "scratch {label} code collection (task-owned, created by this run): {collection}"
            );
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

            let is_multivector = multivector.is_some();
            let mut vector_params = VectorParamsBuilder::new(DIMENSIONS as u64, distance);
            if let Some(multivector) = multivector {
                vector_params = vector_params.multivector_config(multivector);
            }
            client
                .create_collection(
                    CreateCollectionBuilder::new(&collection).vectors_config(vector_params),
                )
                .await
                .with_context(|| format!("create the task-owned {label} collection"))?;

            let outcome = async {
                ensure!(
                    QdrantIndex::ensure_code_collection(&config, &collection, DIMENSIONS)
                        .await
                        .is_err(),
                    "code ensure must refuse a {label} collection"
                );
                ensure!(
                    QdrantIndex::attach_code_read_only_collection(&config, &collection, DIMENSIONS)
                        .await
                        .is_err(),
                    "code read attach must refuse a {label} collection"
                );
                let info = client
                    .collection_info(&collection)
                    .await?
                    .result
                    .with_context(|| format!("missing {label} collection info"))?;
                let vector_params = match info
                    .config
                    .as_ref()
                    .and_then(|config| config.params.as_ref())
                    .and_then(|params| params.vectors_config.as_ref())
                    .and_then(|vectors| vectors.config.as_ref())
                {
                    Some(qdrant_client::qdrant::vectors_config::Config::Params(params)) => params,
                    _ => {
                        return Err(anyhow::anyhow!("the {label} collection shape changed"));
                    }
                };
                ensure!(
                    vector_params.distance == distance as i32,
                    "the refused {label} collection metric must stay unchanged"
                );
                ensure!(
                    vector_params.multivector_config.is_some() == is_multivector,
                    "the refused {label} collection subtype must stay unchanged"
                );
                let (points, _) = QdrantIndex::inspect_collection(&config, &collection, DIMENSIONS)
                    .await?
                    .with_context(|| format!("the {label} collection must still exist"))?;
                ensure!(
                    points == 0,
                    "no code point may be written to a refused {label} collection"
                );
                Ok(())
            }
            .await;

            let cleanup = client
                .delete_collection(&collection)
                .await
                .map(|_| ())
                .with_context(|| format!("delete the task-owned {label} collection"));
            match (outcome, cleanup) {
                (Ok(()), Ok(())) => {}
                (Err(error), Ok(())) => return Err(error),
                (Ok(()), Err(error)) => return Err(error),
                (Err(error), Err(cleanup_error)) => {
                    return Err(anyhow::anyhow!(
                        "{label} refusal failed: {error:#}; cleanup also failed: {cleanup_error:#}"
                    ));
                }
            }
            ensure!(
                QdrantIndex::inspect_collection(&config, &collection, DIMENSIONS)
                    .await?
                    .is_none(),
                "the task-owned {label} collection must be deleted"
            );
            println!("deleted task-owned {label} collection {collection}");
        }

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
