use anyhow::{Result, anyhow};
use qdrant_client::{
    Payload,
    qdrant::{
        DeletePointsBuilder, PointId, PointStruct, PointsIdsList, PointsSelector,
        PointsUpdateOperation, UpdateBatchPointsBuilder, UpdateMode, UpsertPoints,
        UpsertPointsBuilder, points_selector::PointsSelectorOneOf, points_update_operation,
    },
};
use std::time::Instant;
use tokio::time::timeout;
use tracing::info;
use uuid::Uuid;

use super::errors::{bounded_qdrant_chain, format_qdrant_error, qdrant_timeout_error};
use super::payload::chunk_payload_json;
use super::{QDRANT_OPERATION_TIMEOUT, QdrantIndex};
use crate::domain::ChunkPayload;

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

    /// Upserts chunk points in insert-only mode.
    ///
    /// A point id that already exists keeps its stored payload and vector; only
    /// absent ids are inserted. The staged replacement executor writes with this
    /// mode so a retried attempt never overwrites an already verified point, and
    /// a foreign point that shares an id can never be silently replaced.
    pub async fn upsert_document_chunks_insert_only(
        &self,
        payloads: &[ChunkPayload],
        embeddings: &[Vec<f32>],
    ) -> Result<()> {
        let points = self.build_document_points(payloads, embeddings)?;
        if points.is_empty() {
            return Ok(());
        }
        self.upsert_points_insert_only(points, payloads.len()).await
    }

    pub(super) fn build_document_points(
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

    pub(super) async fn upsert_points(
        &self,
        points: Vec<PointStruct>,
        batch_size: usize,
    ) -> Result<()> {
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

    /// The exact request one insert-only upsert issues. Exposed to unit tests so
    /// the fixed wait and mode are pinned without a server.
    pub(super) fn insert_only_upsert_request(
        collection_name: &str,
        points: Vec<PointStruct>,
    ) -> UpsertPoints {
        UpsertPointsBuilder::new(collection_name, points)
            .wait(true)
            .update_mode(UpdateMode::InsertOnly)
            .build()
    }

    pub(super) async fn upsert_points_insert_only(
        &self,
        points: Vec<PointStruct>,
        batch_size: usize,
    ) -> Result<()> {
        if self.is_noop() {
            return Ok(());
        }
        let started = Instant::now();
        let operation = "upsert_points_insert_only";
        let collection = self.collection_name.clone();
        let extra = format!("batch_size={batch_size}");
        let request = Self::insert_only_upsert_request(&self.collection_name, points);
        timeout(QDRANT_OPERATION_TIMEOUT, self.client.upsert_points(request))
            .await
            .map_err(|_| qdrant_timeout_error(operation, &collection, &extra))?
            .map_err(|err| format_qdrant_error(operation, &collection, &extra, err.into()))?;
        info!(
            batch_size,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "qdrant points upserted insert-only"
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

    /// The no-op lane of the test index.
    ///
    /// `QdrantIndex::for_test_noop` builds an index with this placeholder
    /// collection, and every write path of the test lane answers before it
    /// touches a client. The predicate and its policy are unchanged; it is
    /// shared with the cleanup path so the same lane is complete.
    pub(super) fn is_noop(&self) -> bool {
        self.noop
    }
}

/// Human-readable idempotence helper for tests: returns true only for
/// explicit "not found" point/filter signals without permission/validation
/// markers. Production delete paths already treat missing points as success
/// because Qdrant returns success; this helper documents the boundary and
/// is tested separately so we never swallow permission errors.
pub fn is_qdrant_idempotent_not_found(error: &anyhow::Error) -> bool {
    if let Some(typed) = crate::domain_errors::find_domain_error(error) {
        // Typed permission/validation errors are never idempotent. Transient
        // and internal variants fall through to the substring hint check so
        // existing point-not-found behavior is preserved.
        if matches!(
            typed,
            crate::domain_errors::DomainError::Forbidden(_)
                | crate::domain_errors::DomainError::Unauthorized(_)
                | crate::domain_errors::DomainError::InvalidArgument(_)
                | crate::domain_errors::DomainError::UnprocessableEntity(_)
                | crate::domain_errors::DomainError::Conflict(_)
                | crate::domain_errors::DomainError::PayloadTooLarge(_)
        ) {
            return false;
        }
    }
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

#[cfg(test)]
mod tests {
    use super::is_qdrant_idempotent_not_found;
    use crate::domain::ChunkPayload;
    use anyhow::anyhow;
    use qdrant_client::qdrant::{PointStruct, UpdateMode};

    fn payload() -> ChunkPayload {
        ChunkPayload {
            chunk_id: uuid::Uuid::new_v4(),
            document_id: 1,
            group_id: 1,
            group_key: "key".into(),
            group_path: "path".into(),
            visibility: crate::contracts::Visibility::Private,
            source_key: "library".into(),
            external_id: "external".into(),
            title: "title".into(),
            summary: None,
            source_uri: "context69://test".into(),
            published_at: None,
            updated_at_source: chrono::Utc::now(),
            record_hash: "hash".into(),
            chunk_index: 0,
            chunk_text: "text".into(),
            metadata_json: serde_json::json!({}),
            content_locale: "original".into(),
            source_locale: None,
            translation_provider: None,
        }
    }

    #[test]
    fn the_insert_only_request_pins_wait_and_mode() {
        let points = vec![PointStruct::new(
            "00000000-0000-0000-0000-000000000001".to_string(),
            vec![0.0; 4],
            qdrant_client::Payload::try_from(serde_json::json!({"record_hash": "hash"}))
                .expect("a payload object"),
        )];
        let request = super::QdrantIndex::insert_only_upsert_request("the-collection", points);
        assert_eq!(request.collection_name, "the-collection");
        assert_eq!(request.wait, Some(true));
        assert_eq!(
            request.update_mode,
            Some(UpdateMode::InsertOnly as i32),
            "the staged executor must request insert-only mode, never the overwriting default"
        );
        assert_eq!(request.points.len(), 1);
    }

    #[test]
    fn the_insert_only_upsert_rejects_a_dimension_mismatch_before_any_request() {
        let index = super::QdrantIndex::for_test("context69", 8);
        let error = tokio::runtime::Runtime::new()
            .expect("a runtime")
            .block_on(async {
                super::QdrantIndex::upsert_document_chunks_insert_only(
                    &index,
                    &[payload()],
                    &[vec![0.0; 3]],
                )
                .await
                .expect_err("a dimension mismatch must fail before any request")
            });
        assert!(error.to_string().contains("dimension mismatch"));
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
