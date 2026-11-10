//! Indexing batch checkpoint type and pure helpers.
//!
//! Stores a small bounded checkpoint (`next_batch_index`, optional `total_batches`,
//! optional `record_hash`) under the existing `task_items.payload` JSON. The
//! payload primitive is owned by `db::tasks::set_task_item_payload` which is
//! already lease-conditional (`lease_token` + `status='running'`), so a
//! dedicated table would only duplicate lease handling without any additional
//! safety. No full text, embeddings, or document payload ever lives in this
//! checkpoint.
//!
//! All helpers in this module are pure with respect to the tasking database.
//! The persistence driver that wraps these helpers lives in
//! `super::ingest_checkpoint_persistence`.

use anyhow::bail;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::ingest_batches::ChunkBatchIter;
use super::ingest_types::PreparedIngestSection;
use crate::chunking::{ChunkingError, chunk_document_iter_resolved};

/// Stable JSON key reserved for the indexing checkpoint inside the task item
/// payload. Other callers should never read or write this key.
pub const INDEXING_CHECKPOINT_KEY: &str = "indexing_checkpoint";

/// Current checkpoint schema version. A payload with a different version is
/// treated as absent so old formats don't accidentally progress.
pub const INDEXING_CHECKPOINT_VERSION: u32 = 1;

/// Hard upper bound on the JSON size of the serialized checkpoint. Anything
/// larger is rejected by [`payload_with_checkpoint`].
pub const INDEXING_CHECKPOINT_MAX_BYTES: usize = 512;

/// Bounded checkpoint stored under `task_items.payload.indexing_checkpoint`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexingCheckpoint {
    pub v: u32,
    pub next_batch_index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_batches: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_hash: Option<String>,
    /// Identity of the chunk budget that produced the checkpointed batches.
    ///
    /// A checkpoint whose identity does not match the budget the item pinned
    /// describes batches of a different chunking pass, so a resume may not skip
    /// them; the run restarts from batch 0 instead. A legacy checkpoint without
    /// an identity is treated the same way: it is unproven, not trusted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_identity: Option<String>,
    /// Digest of the physical collection and the logical embedding identity
    /// the checkpointed batches were written to.
    ///
    /// A legacy checkpoint without this field is unproven: the run restarts
    /// from batch 0 and reingests under the captured target instead of
    /// trusting batches from a different activated generation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_identity: Option<String>,
}

impl Default for IndexingCheckpoint {
    fn default() -> Self {
        Self {
            v: INDEXING_CHECKPOINT_VERSION,
            next_batch_index: 0,
            total_batches: None,
            record_hash: None,
            budget_identity: None,
            target_identity: None,
        }
    }
}

/// Stable digest of the captured physical collection and logical embedding
/// identity.
///
/// The digest is version-tagged and length-delimited so it cannot collide with
/// another field's history or with a different split of the same strings. An
/// unmanaged runtime has no logical embedding fingerprint; the explicit
/// `unmanaged` tag keeps that case distinct from a managed identity whose
/// fingerprint string happens to be empty.
pub fn checkpoint_target_identity(collection: &str, fingerprint: Option<&str>) -> String {
    use sha2::{Digest, Sha256};

    let mut digest = Sha256::new();
    for part in ["ctx69-checkpoint-target-v1", collection] {
        digest.update((part.len() as u64).to_le_bytes());
        digest.update(part.as_bytes());
        digest.update([0]);
    }
    match fingerprint {
        Some(fingerprint) => {
            digest.update((7u64).to_le_bytes());
            digest.update(b"managed");
            digest.update([0]);
            digest.update((fingerprint.len() as u64).to_le_bytes());
            digest.update(fingerprint.as_bytes());
            digest.update([0]);
        }
        None => {
            digest.update((9u64).to_le_bytes());
            digest.update(b"unmanaged");
            digest.update([0]);
            digest.update(0u64.to_le_bytes());
            digest.update([0]);
        }
    }
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

impl IndexingCheckpoint {
    pub fn reset(
        current_hash: String,
        total_batches: usize,
        budget_identity: String,
        target_identity: String,
    ) -> Self {
        Self {
            v: INDEXING_CHECKPOINT_VERSION,
            next_batch_index: 0,
            total_batches: Some(total_batches),
            record_hash: Some(current_hash),
            budget_identity: Some(budget_identity),
            target_identity: Some(target_identity),
        }
    }

    /// Whether this checkpoint was produced by the same record set, the same
    /// chunk budget and the same captured vector target. Anything unproven does
    /// not match.
    pub fn matches(&self, record_hash: &str, budget_identity: &str, target_identity: &str) -> bool {
        self.record_hash.as_deref() == Some(record_hash)
            && self.budget_identity.as_deref() == Some(budget_identity)
            && self.target_identity.as_deref() == Some(target_identity)
    }

    /// `true` when the parsed checkpoint carries any actual data (non-default).
    fn is_present(&self) -> bool {
        self.total_batches.is_some() || self.record_hash.is_some() || self.target_identity.is_some()
    }
}

/// Parse the checkpoint out of a task item payload. Returns the default
/// (`next_batch_index=0`, no hash) when the key is absent, the value fails to
/// deserialize, or the version field does not match `INDEXING_CHECKPOINT_VERSION`.
pub fn parse_indexing_checkpoint(payload: &Value) -> IndexingCheckpoint {
    payload
        .get(INDEXING_CHECKPOINT_KEY)
        .and_then(|value| serde_json::from_value::<IndexingCheckpoint>(value.clone()).ok())
        .filter(|checkpoint| checkpoint.v == INDEXING_CHECKPOINT_VERSION)
        .unwrap_or_default()
}

pub fn indexing_checkpoint_to_value(checkpoint: &IndexingCheckpoint) -> Value {
    serde_json::to_value(checkpoint).unwrap_or(Value::Null)
}

/// Replace the checkpoint of a payload with a fresh one, preserving every other
/// key.
///
/// Only a proven reset (a changed record set or chunk budget) may use this: it
/// intentionally bypasses the forward-progress rule because restarting from
/// batch 0 is the whole point.
pub fn payload_with_checkpoint_reset(
    payload: &Value,
    checkpoint: &IndexingCheckpoint,
) -> anyhow::Result<Value> {
    write_checkpoint(payload, checkpoint)
}

fn write_checkpoint(payload: &Value, checkpoint: &IndexingCheckpoint) -> anyhow::Result<Value> {
    if checkpoint.v != INDEXING_CHECKPOINT_VERSION {
        bail!("indexing checkpoint version mismatch");
    }
    let checkpoint_value = indexing_checkpoint_to_value(checkpoint);
    let encoded = serde_json::to_string(&checkpoint_value).unwrap_or_default();
    if encoded.len() > INDEXING_CHECKPOINT_MAX_BYTES {
        bail!("indexing checkpoint exceeds bounded size");
    }
    let mut next = payload.clone();
    match &mut next {
        Value::Object(map) => {
            map.insert(INDEXING_CHECKPOINT_KEY.to_string(), checkpoint_value);
        }
        _ => {
            let mut map = serde_json::Map::new();
            map.insert(INDEXING_CHECKPOINT_KEY.to_string(), checkpoint_value);
            next = Value::Object(map);
        }
    }
    Ok(next)
}

/// Preserve every existing payload key (including `section_payload`) and
/// add/update only the small `indexing_checkpoint` key.
///
/// Enforces bounded JSON size (<= [`INDEXING_CHECKPOINT_MAX_BYTES`]),
/// monotonic forward progress, and consistent version field.
pub fn payload_with_checkpoint(
    payload: &Value,
    checkpoint: &IndexingCheckpoint,
) -> anyhow::Result<Value> {
    if checkpoint.v != INDEXING_CHECKPOINT_VERSION {
        bail!("indexing checkpoint version mismatch");
    }
    let existing = parse_indexing_checkpoint(payload);
    if existing.is_present() {
        if checkpoint.next_batch_index <= existing.next_batch_index {
            bail!(
                "indexing checkpoint must advance (current={}, new={})",
                existing.next_batch_index,
                checkpoint.next_batch_index
            );
        }
        if let Some(total) = checkpoint.total_batches
            && checkpoint.next_batch_index > total
        {
            bail!(
                "indexing checkpoint exceeds total batches (next={}, total={})",
                checkpoint.next_batch_index,
                total
            );
        }
    }
    write_checkpoint(payload, checkpoint)
}

/// Hex sha256 across the `record_hash` chain of prepared sections.
pub fn compute_prepared_record_hash(prepared: &[PreparedIngestSection]) -> String {
    let mut hasher = Sha256::new();
    for section in prepared {
        hasher.update(section.normalized.record_hash.as_bytes());
        hasher.update([0u8]);
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Chunk and batch totals of one prepared ingest, measured in a single pass.
///
/// Both counters come from the same `ChunkBatchIter` walk that later produces
/// the batches, so a total can never disagree with the batches it is compared
/// against — including the variable batch sizes caused by the chunk-count and
/// character budgets, and the zero-chunk case where both totals are genuinely
/// `0` rather than unknown.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PreparedIngestTotals {
    pub total_chunks: usize,
    pub total_batches: usize,
}

/// Count the chunks and the batches a prepared set of sections produces, using
/// the same streaming chunking and batching as `persist_document_chunks`.
///
/// The batch total is what the indexing checkpoint advances against and the
/// chunk total is what the progress record publishes; measuring both here is
/// what keeps a resumed run's completed count equal to the batches it skipped.
pub fn count_prepared_ingest_totals(
    prepared: &[PreparedIngestSection],
    chunking: &crate::chunking::ResolvedChunkBudget,
) -> Result<PreparedIngestTotals, ChunkingError> {
    let mut totals = PreparedIngestTotals::default();
    for section in prepared {
        let chunks = chunk_document_iter_resolved(
            0,
            super::FILE_LIBRARY_SOURCE_KEY,
            &section.normalized,
            chunking,
            None,
        )?;
        for batch in ChunkBatchIter::new(chunks) {
            let batch = batch?;
            totals.total_batches += 1;
            totals.total_chunks += batch.len();
        }
    }
    Ok(totals)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunking::ChunkBudget;
    use crate::domain::NormalizedDocument;
    use serde_json::json;

    fn section(record_hash: &str, body: &str) -> PreparedIngestSection {
        PreparedIngestSection {
            index: 0,
            section: super::super::ingest_types::IngestSection {
                section_key: "s".into(),
                section_label: "s".into(),
                title: "t".into(),
                summary: None,
                body_text: body.into(),
                source_uri: None,
                external_id: None,
                published_at: None,
                metadata_json: json!({}),
            },
            normalized: NormalizedDocument {
                external_id: "e".into(),
                title: "t".into(),
                body_text: body.into(),
                source_uri: String::new(),
                summary: None,
                published_at: None,
                updated_at: chrono::Utc::now(),
                record_hash: record_hash.into(),
                metadata_json: json!({}),
            },
        }
    }

    #[test]
    fn parse_returns_default_for_missing_key() {
        let payload = json!({"section_payload": []});
        let checkpoint = parse_indexing_checkpoint(&payload);
        assert_eq!(checkpoint.next_batch_index, 0);
        assert_eq!(checkpoint.record_hash, None);
    }

    #[test]
    fn parse_rejects_unknown_version() {
        let payload = json!({
            "indexing_checkpoint": {"v": 9999, "next_batch_index": 5}
        });
        let checkpoint = parse_indexing_checkpoint(&payload);
        assert_eq!(checkpoint.next_batch_index, 0);
    }

    #[test]
    fn parse_rejects_malformed_payload() {
        let payload = json!({
            "indexing_checkpoint": {"next_batch_index": "not-an-int"}
        });
        let checkpoint = parse_indexing_checkpoint(&payload);
        assert_eq!(checkpoint.next_batch_index, 0);
    }

    #[test]
    fn payload_with_checkpoint_preserves_other_keys() {
        let payload = json!({"section_payload": [1, 2, 3], "file_id": "abc"});
        let checkpoint =
            IndexingCheckpoint::reset("hash".into(), 2, "chars:100:0".into(), "target".into());
        let next = payload_with_checkpoint(&payload, &checkpoint).expect("advance");
        assert_eq!(
            next.get("section_payload")
                .and_then(|v| v.as_array())
                .map(|a| a.len()),
            Some(3)
        );
        assert_eq!(next.get("file_id").and_then(|v| v.as_str()), Some("abc"));
        assert_eq!(
            next.get("indexing_checkpoint")
                .and_then(|v| v.get("next_batch_index"))
                .and_then(|v| v.as_u64()),
            Some(0)
        );
    }

    #[test]
    fn payload_with_checkpoint_rejects_regression() {
        let payload = json!({
            "indexing_checkpoint": {"v": 1, "next_batch_index": 5, "record_hash": "abc"}
        });
        let next = IndexingCheckpoint {
            record_hash: Some("abc".into()),
            next_batch_index: 3,
            ..Default::default()
        };
        let err = payload_with_checkpoint(&payload, &next).expect_err("regression");
        assert!(err.to_string().contains("advance"));
    }

    #[test]
    fn payload_with_checkpoint_rejects_oversize_total() {
        let payload = json!({
            "indexing_checkpoint": {"v": 1, "next_batch_index": 0, "record_hash": "abc", "total_batches": 3}
        });
        let next = IndexingCheckpoint {
            next_batch_index: 4,
            ..IndexingCheckpoint::reset("abc".into(), 3, "x".into(), "target".into())
        };
        let err = payload_with_checkpoint(&payload, &next).expect_err("oversize");
        assert!(err.to_string().contains("total batches"));
    }

    fn resolved_budget(cfg: &ChunkBudget) -> crate::chunking::ResolvedChunkBudget {
        crate::chunking::ChunkingRuntime::new(cfg.clone())
            .resolve(cfg)
            .expect("resolve the static test budget")
    }

    #[test]
    fn count_prepared_ingest_totals_matches_chunking_rules() {
        let cfg = ChunkBudget::chars(crate::chunking::ChunkingConfig {
            max_chars: 100,
            overlap_chars: 0,
        });
        let prepared = vec![section("hash", "hi")];
        assert!(
            count_prepared_ingest_totals(&prepared, &resolved_budget(&cfg))
                .expect("totals")
                .total_batches
                >= 1
        );
    }

    #[test]
    fn count_prepared_ingest_totals_counts_boundaries_across_sections() {
        let cfg = ChunkBudget::chars(crate::chunking::ChunkingConfig {
            max_chars: 100,
            overlap_chars: 0,
        });
        // 33 paragraphs of 60 chars each overflow the 100-char chunking budget
        // pairwise, so chunking yields 33 chunks => 32 + 1 batches.
        let body = vec!["x".repeat(60); 33].join("\n\n");
        let first = count_prepared_ingest_totals(&[section("hash", &body)], &resolved_budget(&cfg))
            .expect("totals");
        assert_eq!(
            first.total_batches, 2,
            "fixture must cross the chunk-count boundary"
        );

        let both = count_prepared_ingest_totals(
            &[section("hash", &body), section("hash-b", "short")],
            &resolved_budget(&cfg),
        )
        .expect("totals");
        assert_eq!(
            both.total_batches,
            first.total_batches + 1,
            "section batches must be summed"
        );
        assert!(
            both.total_chunks > first.total_chunks,
            "chunk totals accumulate across sections too"
        );
    }

    #[test]
    fn totals_count_chunks_and_batches_from_the_same_pass() {
        let cfg = ChunkBudget::chars(crate::chunking::ChunkingConfig {
            max_chars: 100,
            overlap_chars: 0,
        });
        // 33 paragraphs of 60 chars each overflow the 100-char chunking budget
        // pairwise, so chunking yields 33 chunks => 32 + 1 batches.
        let body = vec!["x".repeat(60); 33].join("\n\n");
        let totals =
            count_prepared_ingest_totals(&[section("hash", &body)], &resolved_budget(&cfg))
                .expect("totals");
        assert_eq!(
            totals.total_batches, 2,
            "fixture must cross the count boundary"
        );
        assert_eq!(
            totals.total_chunks, 33,
            "chunk totals come from the batches"
        );

        // A document that produces no chunks has a real total of zero, which
        // is not the same as an unknown total.
        assert_eq!(
            count_prepared_ingest_totals(&[section("hash", "")], &resolved_budget(&cfg))
                .expect("totals"),
            PreparedIngestTotals::default()
        );
    }

    #[test]
    fn compute_prepared_record_hash_changes_with_records() {
        let a = vec![section("hash-a", "a"), section("hash-b", "b")];
        let b = vec![section("hash-x", "a"), section("hash-b", "b")];
        assert_ne!(
            compute_prepared_record_hash(&a),
            compute_prepared_record_hash(&b)
        );
    }

    #[test]
    fn reset_returns_current_hash_and_zero_progress() {
        let checkpoint =
            IndexingCheckpoint::reset("hash".into(), 5, "chars:100:0".into(), "target".into());
        assert_eq!(checkpoint.next_batch_index, 0);
        assert_eq!(checkpoint.record_hash.as_deref(), Some("hash"));
        assert_eq!(checkpoint.total_batches, Some(5));
        assert_eq!(checkpoint.budget_identity.as_deref(), Some("chars:100:0"));
        assert_eq!(checkpoint.target_identity.as_deref(), Some("target"));
    }

    #[test]
    fn target_identity_is_stable_and_distinguishes_unmanaged_from_managed() {
        let unmanaged = checkpoint_target_identity("collection", None);
        assert_eq!(unmanaged.len(), 64);
        assert_eq!(
            unmanaged,
            checkpoint_target_identity("collection", None),
            "the same captured target must produce the same digest"
        );
        assert_ne!(
            unmanaged,
            checkpoint_target_identity("collection", Some("fingerprint"))
        );
        assert_ne!(
            unmanaged,
            checkpoint_target_identity("other-collection", None)
        );
    }

    #[test]
    fn checkpoint_matches_requires_the_captured_target() {
        let checkpoint = IndexingCheckpoint::reset(
            "hash".into(),
            2,
            "chars:100:0".into(),
            checkpoint_target_identity("collection", None),
        );
        assert!(checkpoint.matches(
            "hash",
            "chars:100:0",
            &checkpoint_target_identity("collection", None)
        ));
        assert!(!checkpoint.matches(
            "hash",
            "chars:100:0",
            &checkpoint_target_identity("other", None)
        ));
        let legacy = IndexingCheckpoint {
            target_identity: None,
            ..checkpoint
        };
        assert!(
            !legacy.matches(
                "hash",
                "chars:100:0",
                &checkpoint_target_identity("collection", None)
            ),
            "a legacy checkpoint without a target never proves completion"
        );
    }

    #[test]
    fn checkpoint_target_identity_stays_within_the_bounded_payload() {
        let long_budget = format!("tokens:hf_local:{}:999999:999999", "a".repeat(64));
        let checkpoint = IndexingCheckpoint::reset(
            "b".repeat(64),
            999_999,
            long_budget,
            checkpoint_target_identity("c".repeat(64).as_str(), Some(&"d".repeat(64))),
        );
        let encoded = serde_json::to_string(&indexing_checkpoint_to_value(&checkpoint))
            .expect("serialize the bounded checkpoint");
        assert!(
            encoded.len() <= INDEXING_CHECKPOINT_MAX_BYTES,
            "supported budget and target identities must stay under the checkpoint bound: {}",
            encoded.len()
        );
    }
}
