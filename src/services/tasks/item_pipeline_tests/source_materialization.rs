//! Issue 667 Phase 1 regression coverage for streamed URL sources.
//!
//! A URL item materializes its source into a staged, content-addressed object
//! and records the reference on the item; these cases run the real
//! `process_url` stages (no network) against the shared scratch database and
//! assert the resume, lease-token, and deduplication boundaries. Skipped when
//! `CONTEXT69_TEST_DATABASE_URL` is unset, like the other DB-backed suites.
//!
//! Each boundary lives in its own private sibling module below; this file keeps
//! only the helpers those cases share.

use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::db::{CreateTaskSubmissionRequest, Database};

#[path = "source_materialization_resume.rs"]
mod resume;

#[path = "source_materialization_lease.rs"]
mod lease;

#[path = "source_materialization_duplicate.rs"]
mod duplicate;

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn ingested_options(external_id: &str) -> context69_contracts::IngestOptions {
    context69_contracts::IngestOptions {
        metadata: context69_contracts::CanonicalUploadMetadata {
            external_id: Some(external_id.to_string()),
            source_uri: Some("https://example.com/issue-667.txt".to_string()),
            published_at: None,
            metadata_json: Default::default(),
        },
        translation: None,
        extraction: None,
        source_policy: context69_contracts::SourcePolicy::Retain,
    }
}

fn uploaded_file(
    sha256: &str,
    content: &[u8],
    external_id: &str,
) -> crate::services::library::UploadedLibraryFile {
    crate::services::library::UploadedLibraryFile {
        folder_id: None,
        filename: "issue-667.txt".to_string(),
        media_type: "text/plain".to_string(),
        bytes: bytes::Bytes::copy_from_slice(content),
        declared_sha256: Some(sha256.to_string()),
        options: ingested_options(external_id),
        staged_storage_object_id: None,
    }
}

/// A URL payload as the streaming download stage leaves it: the resolved
/// source metadata plus the staged object reference, never the bytes.
fn staged_url_payload(sha256: &str, external_id: &str) -> Value {
    serde_json::json!({
        "url": "https://example.com/issue-667.txt",
        "options": {
            "metadata": {
                "external_id": external_id,
                "source_uri": "https://example.com/issue-667.txt"
            },
            "source_policy": "retain"
        },
        "download_artifact": {
            "source_url": "https://example.com/issue-667.txt",
            "filename": "issue-667.txt",
            "media_type": "text/plain",
            "sha256": sha256
        }
    })
}

async fn create_url_task(
    db: &Database,
    user_id: i64,
    group_id: i64,
    payloads: &[Value],
    input_storage_object_ids: &[Option<Uuid>],
) -> (Uuid, bool, Vec<Uuid>) {
    db.create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
        task_id: Uuid::new_v4(),
        user_id,
        group_id: Some(group_id),
        kind: "url_batch",
        group_path: Some("test/issue-667"),
        source_key: None,
        payloads,
        input_storage_object_ids: Some(input_storage_object_ids),
        idempotency_key: None,
        request_hash: &format!("issue-667-{}", Uuid::new_v4()),
    })
    .await
    .expect("create url task")
}

async fn claim_item(db: &Database, item_id: Uuid) -> crate::db::ClaimedItem {
    db.claim_items(50)
        .await
        .expect("claim items")
        .into_iter()
        .find(|item| item.id == item_id)
        .expect("claim our item")
}
