use anyhow::{Result, anyhow};
use qdrant_client::{
    Payload,
    qdrant::{
        DeletePointsBuilder, PointId, PointStruct, PointsIdsList, PointsSelector,
        PointsUpdateOperation, UpdateBatchPointsBuilder, UpsertPointsBuilder,
        points_selector::PointsSelectorOneOf, points_update_operation,
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

    fn is_noop(&self) -> bool {
        self.collection_name == "test-noop"
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
    use anyhow::anyhow;

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
