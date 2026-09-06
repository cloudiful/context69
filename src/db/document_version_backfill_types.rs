//! Backfill record shapes (issue 139, phase 4).
//!
//! Shared request/snapshot types for the controlled `file_library`
//! backfill. Scope stays hardcoded to `file_library` in SQL; these types
//! carry identifiers and current document fields only, never bodies.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use crate::db::AuditSamples;

/// Hardcoded backfill scope. Mirrored in SQL; kept here for docs/tests.
pub const FILE_LIBRARY_BACKFILL_SOURCE_KEY: &str = "file_library";

/// One `file_library` document missing a version for its current hash.
#[derive(Debug, Clone, FromRow)]
pub struct FileLibraryMissingVersion {
    pub id: i64,
    pub record_hash: String,
    pub title: String,
    pub summary: Option<String>,
    pub source_uri: String,
    pub published_at: Option<DateTime<Utc>>,
    pub metadata_json: serde_json::Value,
}

/// Locked current fields re-read inside the per-document transaction.
#[derive(Debug, Clone, FromRow)]
pub(crate) struct LockedFileLibraryDocument {
    pub(crate) id: i64,
    pub(crate) record_hash: String,
    pub(crate) title: String,
    pub(crate) summary: Option<String>,
    pub(crate) source_uri: String,
    pub(crate) published_at: Option<DateTime<Utc>>,
    pub(crate) metadata_json: serde_json::Value,
}

/// Ordered chunk row for body reconstruction (matches
/// `get_document_chunks.sql` column shape).
#[derive(Debug, Clone, FromRow)]
pub(crate) struct BackfillChunkRow {
    pub(crate) id: Uuid,
    pub(crate) chunk_index: i32,
    pub(crate) chunk_text: String,
}

/// Deterministic read-only preflight over the `file_library` scope.
/// `eligible_ids` holds the full ordered id list (no bodies); `samples`
/// holds bounded id samples per verdict for operator preview.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackfillPreflight {
    pub scanned: usize,
    pub eligible: usize,
    pub zero_chunks: usize,
    pub blank_body: usize,
    pub non_contiguous_or_duplicate: usize,
    pub hash_mismatch: usize,
    pub eligible_ids: Vec<i64>,
    pub samples: AuditSamples,
    pub truncated: bool,
    pub page_size: i64,
    pub max_documents: i64,
}

/// One skipped document (validation failed, no write, rolled back).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackfillSkippedDoc {
    pub id: i64,
    pub reason: String,
}

/// One document whose transaction failed with an operational error.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackfillErrorDoc {
    pub id: i64,
    pub error: String,
}

/// Operator-auditable apply summary. Identifier lists only; bodies and
/// chunk texts are never included.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackfillApplySummary {
    pub scanned: usize,
    pub inserted: usize,
    pub already_fixed: usize,
    pub skipped: usize,
    pub errored: usize,
    pub inserted_ids: Vec<i64>,
    pub already_fixed_ids: Vec<i64>,
    pub skipped_docs: Vec<BackfillSkippedDoc>,
    pub error_docs: Vec<BackfillErrorDoc>,
}
