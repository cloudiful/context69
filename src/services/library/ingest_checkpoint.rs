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

use super::ingest_types::PreparedIngestSection;
use crate::chunking::chunk_document_iter;

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
}

impl Default for IndexingCheckpoint {
    fn default() -> Self {
        Self {
            v: INDEXING_CHECKPOINT_VERSION,
            next_batch_index: 0,
            total_batches: None,
            record_hash: None,
        }
    }
}

impl IndexingCheckpoint {
    pub fn reset(current_hash: String, total_batches: usize) -> Self {
        Self {
            v: INDEXING_CHECKPOINT_VERSION,
            next_batch_index: 0,
            total_batches: Some(total_batches),
            record_hash: Some(current_hash),
        }
    }

    /// `true` when the parsed checkpoint carries any actual data (non-default).
    fn is_present(&self) -> bool {
        self.total_batches.is_some() || self.record_hash.is_some()
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
    let checkpoint_value = indexing_checkpoint_to_value(checkpoint);
    let encoded = serde_json::to_string(&checkpoint_value).unwrap_or_default();
    if encoded.len() > INDEXING_CHECKPOINT_MAX_BYTES {
        bail!("indexing checkpoint exceeds bounded size");
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

/// Hex sha256 of the JSON-serialized section payload. Exposed for callers
/// that want to verify the same hash the indexing pipeline computes against
/// the `section_payload` they persisted themselves.
#[allow(dead_code)]
pub fn compute_section_payload_record_hash(section_payload: &Value) -> String {
    let json = serde_json::to_string(section_payload).unwrap_or_default();
    let digest = Sha256::digest(json.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
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

/// Estimate the total number of embedding batches a prepared set of sections
/// would produce. Mirrors the batching rules of `persist_document_chunks`.
pub fn estimate_total_batches(
    prepared: &[PreparedIngestSection],
    chunking: &crate::chunking::ChunkingConfig,
) -> usize {
    use super::ingest_batches::{MAX_BATCH_CHARS, MAX_BATCH_CHUNKS};
    let mut total = 0usize;
    for section in prepared {
        let mut chunks = chunk_document_iter(
            0,
            super::FILE_LIBRARY_SOURCE_KEY,
            &section.normalized,
            chunking,
        );
        let mut pending: Option<crate::domain::DocumentChunk> = None;
        loop {
            let mut batch = Vec::with_capacity(MAX_BATCH_CHUNKS);
            let mut batch_chars = 0usize;
            while batch.len() < MAX_BATCH_CHUNKS {
                let next = pending.take().or_else(|| chunks.next());
                let Some(chunk) = next else {
                    break;
                };
                let chunk_chars = chunk.text.chars().count();
                if !batch.is_empty() && batch_chars + chunk_chars > MAX_BATCH_CHARS {
                    pending = Some(chunk);
                    break;
                }
                batch_chars += chunk_chars;
                batch.push(chunk);
            }
            if batch.is_empty() {
                break;
            }
            total += 1;
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let checkpoint = IndexingCheckpoint::reset("hash".into(), 2);
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
        let mut next = IndexingCheckpoint::default();
        next.record_hash = Some("abc".into());
        next.next_batch_index = 3;
        let err = payload_with_checkpoint(&payload, &next).expect_err("regression");
        assert!(err.to_string().contains("advance"));
    }

    #[test]
    fn payload_with_checkpoint_rejects_oversize_total() {
        let payload = json!({
            "indexing_checkpoint": {"v": 1, "next_batch_index": 0, "record_hash": "abc", "total_batches": 3}
        });
        let mut next = IndexingCheckpoint::reset("abc".into(), 3);
        next.next_batch_index = 4;
        let err = payload_with_checkpoint(&payload, &next).expect_err("oversize");
        assert!(err.to_string().contains("total batches"));
    }

    #[test]
    fn estimate_total_batches_matches_chunking_rules() {
        let cfg = crate::chunking::ChunkingConfig {
            max_chars: 100,
            overlap_chars: 0,
        };
        let prepared = vec![section("hash", "hi")];
        assert!(estimate_total_batches(&prepared, &cfg) >= 1);
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
        let checkpoint = IndexingCheckpoint::reset("hash".into(), 5);
        assert_eq!(checkpoint.next_batch_index, 0);
        assert_eq!(checkpoint.record_hash.as_deref(), Some("hash"));
        assert_eq!(checkpoint.total_batches, Some(5));
    }
}
