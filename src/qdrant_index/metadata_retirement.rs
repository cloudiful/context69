//! Narrow Qdrant payload retirement (issue #910).
//!
//! The payload-sync worker removes exactly one nested payload key from
//! explicitly enumerated original and translated point ids. It never writes
//! text, locale, record hash, model, vector, or library payload fields, and it
//! never recreates a missing point.
//!
//! Original points are verified read-only before any write: every expected
//! point must exist and carry the exact `document_id` and `record_hash`. Only
//! already-absent historical translated points are allowed to be a counted
//! no-op, which keeps a wrong or empty same-dimension collection from reaching
//! the strip.

use std::collections::HashMap;
use std::time::Instant;

use anyhow::{Context, Result};
use qdrant_client::qdrant::{
    GetPointsBuilder, PointId, PointsIdsList, PointsSelector, PointsUpdateOperation,
    UpdateBatchPointsBuilder, points_selector, points_update_operation,
};
use tokio::time::timeout;
use tracing::info;
use uuid::Uuid;

use super::{QDRANT_OPERATION_TIMEOUT, QdrantIndex, format_qdrant_error, qdrant_timeout_error};

/// The only payload key this module removes: the retired enrichment namespace
/// inside the metadata mirror.
pub const EXTRACTION_PAYLOAD_KEY: &str = "metadata_index.extraction";

/// Bounded number of point ids per Qdrant batch.
pub const RETIREMENT_BATCH_SIZE: usize = 100;

/// One original point that must exist with this exact identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OriginalPointExpectation {
    pub point_id: Uuid,
    pub document_id: i64,
    pub record_hash: String,
}

/// Result of one retirement call.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PayloadRetirementOutcome {
    /// Deduplicated original point ids targeted by the delete.
    pub original_targeted: usize,
    /// Deduplicated translated point ids targeted by the delete.
    pub translated_targeted: usize,
    /// Translated ids absent from the collection; nothing is recreated.
    pub translated_missing: usize,
}

impl PayloadRetirementOutcome {
    pub fn targeted(self) -> usize {
        self.original_targeted + self.translated_targeted
    }
}

impl QdrantIndex {
    /// Verify every expected original point exists with the exact identity.
    ///
    /// A missing original point, a wrong `document_id`, or a different
    /// `record_hash` aborts. An empty collection yields a missing count equal
    /// to the expectation count, so it can never pass.
    pub async fn verify_original_points(
        &self,
        expected: &[OriginalPointExpectation],
    ) -> Result<()> {
        if expected.is_empty() {
            anyhow::bail!("document has no original points to verify");
        }
        let mut matched = 0usize;
        for batch in expected.chunks(RETIREMENT_BATCH_SIZE) {
            let ids: Vec<PointId> = batch
                .iter()
                .map(|entry| PointId::from(entry.point_id.to_string()))
                .collect();
            let payloads = self.read_payloads(&ids).await?;
            for entry in batch {
                let Some(payload) = payloads.get(&entry.point_id) else {
                    anyhow::bail!(
                        "original point {} is missing from collection '{}'",
                        entry.point_id,
                        self.collection_name
                    );
                };
                let document_id = payload
                    .get("document_id")
                    .and_then(serde_json::Value::as_i64);
                let record_hash = payload
                    .get("record_hash")
                    .and_then(serde_json::Value::as_str);
                if document_id != Some(entry.document_id) {
                    anyhow::bail!(
                        "original point {} belongs to a different document",
                        entry.point_id
                    );
                }
                if record_hash != Some(entry.record_hash.as_str()) {
                    anyhow::bail!(
                        "original point {} belongs to a different source generation",
                        entry.point_id
                    );
                }
                matched += 1;
            }
        }
        if matched != expected.len() {
            anyhow::bail!("original point verification was incomplete");
        }
        Ok(())
    }

    /// Remove [`EXTRACTION_PAYLOAD_KEY`] from the given original and translated
    /// points. Originals must already be verified. Translated points that no
    /// longer exist are reported as missing and are never recreated.
    pub async fn retire_extraction_payload(
        &self,
        originals: &[Uuid],
        translations: &[Uuid],
    ) -> Result<PayloadRetirementOutcome> {
        let originals = dedup(originals);
        let translations = dedup(translations);
        if originals.is_empty() && translations.is_empty() {
            return Ok(PayloadRetirementOutcome::default());
        }

        let mut outcome = PayloadRetirementOutcome {
            original_targeted: originals.len(),
            translated_targeted: translations.len(),
            translated_missing: 0,
        };

        // Originals are guaranteed by `verify_original_points`; deleting from a
        // vanished original is still a hard failure rather than a silent no-op.
        for batch in originals.chunks(RETIREMENT_BATCH_SIZE) {
            if self.count_present(batch).await? != batch.len() {
                anyhow::bail!("an original point disappeared before the payload delete");
            }
            self.delete_key(batch).await?;
        }
        for batch in translations.chunks(RETIREMENT_BATCH_SIZE) {
            let present = self.count_present(batch).await?;
            outcome.translated_missing += batch.len().saturating_sub(present);
            if present == 0 {
                continue;
            }
            let existing = batch.to_vec();
            self.delete_key(&existing).await?;
        }
        Ok(outcome)
    }

    async fn delete_key(&self, batch: &[Uuid]) -> Result<()> {
        let started = Instant::now();
        let operation = "delete_metadata_payload";
        let collection = self.collection_name.clone();
        let extra = format!("point_count={}", batch.len());
        let operations = extraction_delete_operations(batch);
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
            batch_size = batch.len(),
            elapsed_ms = started.elapsed().as_millis() as u64,
            "qdrant extraction payload retired"
        );
        Ok(())
    }

    async fn count_present(&self, batch: &[Uuid]) -> Result<usize> {
        let ids: Vec<PointId> = batch
            .iter()
            .map(|id| PointId::from(id.to_string()))
            .collect();
        Ok(self.read_payloads(&ids).await?.len())
    }

    async fn read_payloads(&self, ids: &[PointId]) -> Result<HashMap<Uuid, serde_json::Value>> {
        let operation = "get_points";
        let collection = self.collection_name.clone();
        let extra = format!("point_count={}", ids.len());
        let response = timeout(
            QDRANT_OPERATION_TIMEOUT,
            self.client.get_points(
                GetPointsBuilder::new(&self.collection_name, ids.to_vec())
                    .with_payload(true)
                    .with_vectors(false),
            ),
        )
        .await
        .map_err(|_| qdrant_timeout_error(operation, &collection, &extra))?
        .map_err(|err| format_qdrant_error(operation, &collection, &extra, err.into()))?;
        let mut payloads = HashMap::new();
        for point in response.result {
            let Some(point_id) = point.id else {
                continue;
            };
            if let Ok(id) = super::point_id_to_uuid(point_id) {
                let payload: qdrant_client::Payload = point.payload.into();
                let payload = payload
                    .deserialize::<serde_json::Value>()
                    .context("decode point payload for verification")?;
                payloads.insert(id, payload);
            }
        }
        Ok(payloads)
    }
}

/// Deduplicate while keeping the first occurrence order.
fn dedup(ids: &[Uuid]) -> Vec<Uuid> {
    let mut seen = std::collections::HashSet::new();
    ids.iter().copied().filter(|id| seen.insert(*id)).collect()
}

/// Build the exact delete operations issued for a batch. Exposed to unit tests
/// in this module so the fixed key and points selector are asserted directly.
pub(super) fn extraction_delete_operations(ids: &[Uuid]) -> Vec<PointsUpdateOperation> {
    let point_ids = ids
        .iter()
        .map(|id| PointId::from(id.to_string()))
        .collect::<Vec<_>>();
    vec![PointsUpdateOperation {
        operation: Some(points_update_operation::Operation::DeletePayload(
            points_update_operation::DeletePayload {
                keys: vec![EXTRACTION_PAYLOAD_KEY.to_string()],
                points_selector: Some(PointsSelector {
                    points_selector_one_of: Some(points_selector::PointsSelectorOneOf::Points(
                        PointsIdsList { ids: point_ids },
                    )),
                }),
                shard_key_selector: None,
            },
        )),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delete_operation_carries_only_the_fixed_key_and_selector() {
        let first = Uuid::nil();
        let second = Uuid::from_u128(7);
        let operations = extraction_delete_operations(&[first, second]);
        assert_eq!(operations.len(), 1);
        let Some(points_update_operation::Operation::DeletePayload(payload)) =
            operations[0].operation.as_ref()
        else {
            panic!("expected a payload-key delete");
        };
        assert_eq!(payload.keys, vec![EXTRACTION_PAYLOAD_KEY.to_string()]);
        assert_eq!(payload.shard_key_selector, None);
        let selector = payload.points_selector.as_ref().expect("selector");
        let Some(points_selector::PointsSelectorOneOf::Points(list)) =
            &selector.points_selector_one_of
        else {
            panic!("expected explicit point list");
        };
        let rendered: Vec<String> = list
            .ids
            .iter()
            .map(|id| match id.point_id_options.as_ref() {
                Some(qdrant_client::qdrant::point_id::PointIdOptions::Uuid(value)) => value.clone(),
                Some(qdrant_client::qdrant::point_id::PointIdOptions::Num(value)) => {
                    value.to_string()
                }
                None => String::new(),
            })
            .collect();
        assert_eq!(rendered, vec![first.to_string(), second.to_string()]);
    }

    #[test]
    fn the_retired_key_is_scoped_to_the_metadata_namespace() {
        assert_eq!(EXTRACTION_PAYLOAD_KEY, "metadata_index.extraction");
        assert!(!EXTRACTION_PAYLOAD_KEY.contains("metadata_index.stock"));
    }

    #[tokio::test]
    async fn empty_original_expectation_fails_before_any_request() {
        let index = QdrantIndex::for_test("context69", 8);
        let error = index
            .verify_original_points(&[])
            .await
            .expect_err("empty expectation must fail");
        assert!(error.to_string().contains("no original points"));
    }

    #[test]
    fn dedup_keeps_first_occurrence() {
        let first = Uuid::from_u128(1);
        let second = Uuid::from_u128(2);
        assert_eq!(dedup(&[first, second, first]), vec![first, second]);
    }
}
