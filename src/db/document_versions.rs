//! Version-snapshot helpers for library business-fields updates (issue 139).
//!
//! `update_library_document_business_fields` changes `documents.record_hash`
//! without going through `upsert_document`, so it must persist a matching
//! `document_versions` row in the same transaction. Same-hash publishes stay
//! a no-op for versions; changed hashes reconstruct the complete body from
//! ordered chunks and fail closed when no valid body exists.

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};

use crate::domain::{ChunkPayload, SourceRecord};
use crate::normalize::{normalize_body, normalize_record};

use super::ChunkRow;

#[derive(Debug, Clone, FromRow)]
struct VersionBasisRow {
    record_hash: String,
    title: String,
    summary: Option<String>,
}

/// Ensure a matching version snapshot when a business-fields update changes
/// the record hash. Call inside the same transaction as the parent/chunk and
/// metadata-index updates. Returns `true` when a snapshot was ensured and
/// `false` for same-hash or missing-document no-ops.
pub(super) async fn ensure_library_version_in_transaction(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    document_id: i64,
    payload: &ChunkPayload,
) -> Result<bool> {
    let basis = sqlx::query_file_as!(
        VersionBasisRow,
        "src/sql/db/documents/get_document_version_basis_for_update.sql",
        document_id
    )
    .fetch_optional(&mut **tx)
    .await
    .context("load document version basis")?;

    let Some(basis) = basis else {
        return Ok(false);
    };
    if basis.record_hash == payload.record_hash {
        return Ok(false);
    }

    // Reconstruct the complete body from ordered chunks. Never use
    // `payload.chunk_text` here: the extraction publisher only carries the
    // first chunk, which would persist a partial snapshot.
    let chunks = sqlx::query_file_as!(
        ChunkRow,
        "src/sql/db/documents/get_document_chunks.sql",
        document_id
    )
    .fetch_all(&mut **tx)
    .await
    .context("load ordered document chunks for version snapshot")?;

    if chunks.is_empty() {
        anyhow::bail!(
            "cannot snapshot document version for changed record_hash: document {document_id} has no chunks"
        );
    }
    let body_text = chunks
        .iter()
        .map(|chunk| chunk.chunk_text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    if body_text.trim().is_empty() {
        anyhow::bail!(
            "cannot snapshot document version for changed record_hash: document {document_id} has no valid body text"
        );
    }

    sqlx::query_file!(
        "src/sql/db/documents/insert_document_version.sql",
        document_id,
        payload.record_hash,
        basis.title,
        basis.summary,
        body_text,
        payload.source_uri,
        payload.published_at,
        payload.metadata_json
    )
    .execute(&mut **tx)
    .await
    .context("insert document version snapshot")?;

    Ok(true)
}

// --- Read-only repair-preview audit (issue 139, phase 3) -------------------
// SELECT-only helpers for previewing documents whose current
// `documents.record_hash` lacks a matching `document_versions` row. No
// helper here writes, migrates, or retries; the operator binary pages
// through these queries and classifies each document in application code.

/// One document page row for the missing-version audit.
#[derive(Debug, Clone, FromRow)]
pub struct MissingVersionDocument {
    /// Document id; the paging cursor (`ORDER BY id`).
    pub id: i64,
    /// Currently stored hash that has no matching version row.
    pub record_hash: String,
    /// Current title used for application-side hash recomputation.
    pub title: String,
    /// Current summary used for application-side hash recomputation.
    pub summary: Option<String>,
    /// Current source URI used for application-side hash recomputation.
    pub source_uri: String,
    /// Current metadata used for application-side hash recomputation.
    pub metadata_json: serde_json::Value,
}

/// One ordered chunk for audit classification (no body text leaves the DB
/// except through the bounded reconstruct-and-hash path below).
#[derive(Debug, Clone)]
pub struct AuditChunk {
    pub chunk_index: i32,
    pub chunk_text: String,
}

/// Audit verdict for a single missing-version document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditVerdict {
    /// Contiguous zero-based chunks, non-blank body, application hash matches.
    Eligible,
    /// No chunk rows at all.
    ZeroChunks,
    /// Chunks exist but the normalized body is empty.
    BlankBody,
    /// Chunk indexes are gapped, misaligned, or duplicated.
    NonContiguousOrDuplicate,
    /// Body shape is fine but the recomputed application hash diverges.
    HashMismatch,
}

impl AuditVerdict {
    /// Stable machine-readable label for JSON summaries.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Eligible => "eligible",
            Self::ZeroChunks => "zero_chunks",
            Self::BlankBody => "blank_body",
            Self::NonContiguousOrDuplicate => "non_contiguous_or_duplicate",
            Self::HashMismatch => "hash_mismatch",
        }
    }
}

/// Machine-readable audit summary. Holds identifier samples only; document
/// bodies and chunk texts are never included.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditSummary {
    pub scanned: usize,
    pub eligible: usize,
    pub zero_chunks: usize,
    pub blank_body: usize,
    pub non_contiguous_or_duplicate: usize,
    pub hash_mismatch: usize,
    pub samples: AuditSamples,
    pub truncated: bool,
    pub page_size: i64,
    pub max_documents: i64,
}

/// Bounded document-id samples per verdict (ids only, no bodies).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuditSamples {
    #[serde(default)]
    pub eligible: Vec<i64>,
    #[serde(default)]
    pub zero_chunks: Vec<i64>,
    #[serde(default)]
    pub blank_body: Vec<i64>,
    #[serde(default)]
    pub non_contiguous_or_duplicate: Vec<i64>,
    #[serde(default)]
    pub hash_mismatch: Vec<i64>,
}

impl AuditSummary {
    fn push_sample(&mut self, verdict: AuditVerdict, document_id: i64, sample_size: usize) {
        if sample_size == 0 {
            return;
        }
        let slot = match verdict {
            AuditVerdict::Eligible => &mut self.samples.eligible,
            AuditVerdict::ZeroChunks => &mut self.samples.zero_chunks,
            AuditVerdict::BlankBody => &mut self.samples.blank_body,
            AuditVerdict::NonContiguousOrDuplicate => &mut self.samples.non_contiguous_or_duplicate,
            AuditVerdict::HashMismatch => &mut self.samples.hash_mismatch,
        };
        if slot.len() < sample_size {
            slot.push(document_id);
        }
    }
}

/// Classify one missing-version document using application normalization and
/// hash semantics (`normalize_record`). Priority is deterministic:
/// zero chunks, then contiguity/duplicates, then blank body, then hash
/// match/mismatch.
pub fn classify_audit_document(
    document: &MissingVersionDocument,
    chunks: &[AuditChunk],
) -> AuditVerdict {
    if chunks.is_empty() {
        return AuditVerdict::ZeroChunks;
    }
    if !audit_chunks_contiguous(chunks) {
        return AuditVerdict::NonContiguousOrDuplicate;
    }
    let raw_body = chunks
        .iter()
        .map(|chunk| chunk.chunk_text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    if normalize_body(&raw_body).trim().is_empty() {
        return AuditVerdict::BlankBody;
    }
    if recomputed_audit_hash(document, &raw_body) == document.record_hash {
        AuditVerdict::Eligible
    } else {
        AuditVerdict::HashMismatch
    }
}

/// Check zero-based contiguity on sorted indexes so gaps, a non-zero start,
/// and duplicate indexes all fail. Sorting keeps unordered test inputs and
/// ordered DB pages equivalent.
fn audit_chunks_contiguous(chunks: &[AuditChunk]) -> bool {
    let mut indexes: Vec<i32> = chunks.iter().map(|chunk| chunk.chunk_index).collect();
    indexes.sort_unstable();
    for (position, index) in indexes.iter().enumerate() {
        let Ok(expected) = i32::try_from(position) else {
            return false;
        };
        if *index != expected {
            return false;
        }
    }
    true
}

/// Recompute the application hash for the reconstructed body. Mirrors
/// `refresh_metadata_for_file`: the ordered chunks are joined with `"\n"`
/// and fed through `normalize_record` with the current document fields, so
/// whitespace/`normalize_body` semantics match the write path. The stored
/// `metadata_json` value is hashed via its `to_string()` form exactly as the
/// application does; a jsonb key-order round-trip that changes that string
/// surfaces as `HashMismatch` for operator review rather than silent repair.
fn recomputed_audit_hash(document: &MissingVersionDocument, raw_body: &str) -> String {
    normalize_record(SourceRecord {
        external_id: format!("audit-{}", document.id),
        title: document.title.clone(),
        body_text: raw_body.to_string(),
        source_uri: document.source_uri.clone(),
        summary: document.summary.clone(),
        published_at: None,
        updated_at: Utc::now(),
        metadata_json: document.metadata_json.clone(),
    })
    .record_hash
}

/// Read-only page of missing-version documents ordered by id. `after_id` is
/// the last id from the previous page (`None` for the first page).
pub async fn list_missing_version_page(
    pool: &PgPool,
    after_id: Option<i64>,
    limit: i64,
) -> Result<Vec<MissingVersionDocument>> {
    let rows = sqlx::query_file_as!(
        MissingVersionDocument,
        "src/sql/db/documents/list_missing_document_versions.sql",
        after_id,
        limit,
    )
    .fetch_all(pool)
    .await
    .context("list missing document versions page")?;
    Ok(rows)
}

/// Bounded read-only audit over missing-version documents. Pages by document
/// id, reconstructs each body from ordered chunks, and classifies with
/// [`classify_audit_document`]. SELECT-only; performs no writes, migrations,
/// or retries. Bodies are hashed in memory and never returned.
pub async fn audit_missing_versions(
    pool: &PgPool,
    page_size: i64,
    max_documents: i64,
    sample_size: usize,
) -> Result<AuditSummary> {
    let page_size = page_size.clamp(1, 1000);
    let max_documents = max_documents.clamp(1, 100_000);
    let mut summary = AuditSummary {
        scanned: 0,
        eligible: 0,
        zero_chunks: 0,
        blank_body: 0,
        non_contiguous_or_duplicate: 0,
        hash_mismatch: 0,
        samples: AuditSamples::default(),
        truncated: false,
        page_size,
        max_documents,
    };
    let mut after_id: Option<i64> = None;
    loop {
        let remaining = max_documents - i64::try_from(summary.scanned).unwrap_or(i64::MAX);
        if remaining <= 0 {
            summary.truncated = true;
            break;
        }
        let limit = page_size.min(remaining);
        let page = list_missing_version_page(pool, after_id, limit).await?;
        if page.is_empty() {
            break;
        }
        for document in &page {
            let chunk_rows = sqlx::query_file_as!(
                ChunkRow,
                "src/sql/db/documents/get_document_chunks.sql",
                document.id,
            )
            .fetch_all(pool)
            .await
            .context("load ordered chunks for audit")?;
            let chunks = chunk_rows
                .into_iter()
                .map(|row| AuditChunk {
                    chunk_index: row.chunk_index,
                    chunk_text: row.chunk_text,
                })
                .collect::<Vec<_>>();
            let verdict = classify_audit_document(document, &chunks);
            match verdict {
                AuditVerdict::Eligible => summary.eligible += 1,
                AuditVerdict::ZeroChunks => summary.zero_chunks += 1,
                AuditVerdict::BlankBody => summary.blank_body += 1,
                AuditVerdict::NonContiguousOrDuplicate => {
                    summary.non_contiguous_or_duplicate += 1;
                }
                AuditVerdict::HashMismatch => summary.hash_mismatch += 1,
            }
            summary.push_sample(verdict, document.id, sample_size);
            summary.scanned += 1;
            after_id = Some(document.id);
            if i64::try_from(summary.scanned).unwrap_or(i64::MAX) >= max_documents {
                break;
            }
        }
        if i64::try_from(summary.scanned).unwrap_or(i64::MAX) >= max_documents {
            // Peek deterministically: another page exists only when the last
            // fetch was full; mark truncation without scanning further.
            if i64::try_from(page.len()).unwrap_or(0) >= limit {
                let next = list_missing_version_page(pool, after_id, 1).await?;
                summary.truncated = !next.is_empty();
            }
            break;
        }
        if i64::try_from(page.len()).unwrap_or(0) < limit {
            break;
        }
    }
    Ok(summary)
}
