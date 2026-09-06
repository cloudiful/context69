//! Read-only `file_library` backfill preflight (issue 139, phase 4).
//!
//! Deterministic scope scan plus the apply-mode guards. SELECT-only;
//! never writes, migrates, retries, or falls back to `.env` for writes.

use anyhow::{Context, Result};
use sqlx::PgPool;

use super::types::{BackfillPreflight, FileLibraryMissingVersion};
use crate::db::{AuditChunk, AuditVerdict, MissingVersionDocument, classify_audit_document};

/// Resolve the apply-mode database URL. Apply mode accepts only an
/// explicit `--database-url` value; repository `.env` / `DATABASE_URL` /
/// app-config fallback is disabled for writes by construction (this
/// helper never reads the environment).
pub fn resolve_apply_database_url(cli_database_url: Option<&str>) -> Result<String> {
    match cli_database_url
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(url) => Ok(url.to_string()),
        None => anyhow::bail!(
            "--apply requires an explicit --database-url; repository .env / DATABASE_URL fallback is disabled for writes"
        ),
    }
}

/// Guard the apply path: abort without writes when the fresh preflight is
/// truncated, the eligible count drifts from the operator-supplied
/// expected count, or any candidate is not safely repairable.
pub fn check_backfill_preflight(
    preflight: &BackfillPreflight,
    expected_eligible: usize,
) -> Result<()> {
    if preflight.truncated {
        anyhow::bail!(
            "backfill preflight is truncated (scanned {} of at least {}); refusing to write with an incomplete scope",
            preflight.scanned,
            preflight.max_documents
        );
    }
    if preflight.eligible != expected_eligible {
        anyhow::bail!(
            "backfill eligible count drift: expected {expected_eligible}, found {} (scanned {}); refusing to write",
            preflight.eligible,
            preflight.scanned
        );
    }
    let unsafe_count = preflight.zero_chunks
        + preflight.blank_body
        + preflight.non_contiguous_or_duplicate
        + preflight.hash_mismatch;
    if unsafe_count > 0 {
        anyhow::bail!(
            "backfill preflight found {unsafe_count} not safely repairable candidate(s) \
             (zero_chunks={} blank_body={} non_contiguous_or_duplicate={} hash_mismatch={}); refusing to write",
            preflight.zero_chunks,
            preflight.blank_body,
            preflight.non_contiguous_or_duplicate,
            preflight.hash_mismatch
        );
    }
    Ok(())
}

/// Read-only page of `file_library` documents missing a version, ordered
/// by id.
pub async fn list_missing_file_library_page(
    pool: &PgPool,
    after_id: Option<i64>,
    limit: i64,
) -> Result<Vec<FileLibraryMissingVersion>> {
    let rows = sqlx::query_file_as!(
        FileLibraryMissingVersion,
        "src/sql/db/documents/list_missing_file_library_document_versions.sql",
        after_id,
        limit,
    )
    .fetch_all(pool)
    .await
    .context("list missing file_library document versions page")?;
    Ok(rows)
}

fn audit_view(document: &FileLibraryMissingVersion) -> MissingVersionDocument {
    MissingVersionDocument {
        id: document.id,
        record_hash: document.record_hash.clone(),
        title: document.title.clone(),
        summary: document.summary.clone(),
        source_uri: document.source_uri.clone(),
        metadata_json: document.metadata_json.clone(),
    }
}

fn push_sample(samples: &mut crate::db::AuditSamples, verdict: AuditVerdict, id: i64, cap: usize) {
    if cap == 0 {
        return;
    }
    let slot = match verdict {
        AuditVerdict::Eligible => &mut samples.eligible,
        AuditVerdict::ZeroChunks => &mut samples.zero_chunks,
        AuditVerdict::BlankBody => &mut samples.blank_body,
        AuditVerdict::NonContiguousOrDuplicate => &mut samples.non_contiguous_or_duplicate,
        AuditVerdict::HashMismatch => &mut samples.hash_mismatch,
    };
    if slot.len() < cap {
        slot.push(id);
    }
}

/// Bounded read-only preflight over the `file_library` scope. Pages by
/// id, reconstructs each body from ordered chunks, and classifies with
/// the same [`classify_audit_document`] semantics as the phase 3 audit.
pub async fn preflight_file_library_backfill(
    pool: &PgPool,
    page_size: i64,
    max_documents: i64,
    sample_size: usize,
) -> Result<BackfillPreflight> {
    let page_size = page_size.clamp(1, 1000);
    let max_documents = max_documents.clamp(1, 100_000);
    let mut preflight = BackfillPreflight {
        scanned: 0,
        eligible: 0,
        zero_chunks: 0,
        blank_body: 0,
        non_contiguous_or_duplicate: 0,
        hash_mismatch: 0,
        eligible_ids: Vec::new(),
        samples: crate::db::AuditSamples::default(),
        truncated: false,
        page_size,
        max_documents,
    };
    let mut after_id: Option<i64> = None;
    loop {
        let remaining = max_documents - i64::try_from(preflight.scanned).unwrap_or(i64::MAX);
        if remaining <= 0 {
            preflight.truncated = true;
            break;
        }
        let limit = page_size.min(remaining);
        let page = list_missing_file_library_page(pool, after_id, limit).await?;
        if page.is_empty() {
            break;
        }
        for document in &page {
            let chunk_rows = sqlx::query_file_as!(
                super::types::BackfillChunkRow,
                "src/sql/db/documents/get_document_chunks.sql",
                document.id,
            )
            .fetch_all(pool)
            .await
            .context("load ordered chunks for backfill preflight")?;
            debug_assert!(
                chunk_rows.iter().all(|row| !row.id.is_nil()),
                "chunk ids must be real UUIDs"
            );
            let chunks = chunk_rows
                .into_iter()
                .map(|row| AuditChunk {
                    chunk_index: row.chunk_index,
                    chunk_text: row.chunk_text,
                })
                .collect::<Vec<_>>();
            let verdict = classify_audit_document(&audit_view(document), &chunks);
            match verdict {
                AuditVerdict::Eligible => {
                    preflight.eligible += 1;
                    preflight.eligible_ids.push(document.id);
                }
                AuditVerdict::ZeroChunks => preflight.zero_chunks += 1,
                AuditVerdict::BlankBody => preflight.blank_body += 1,
                AuditVerdict::NonContiguousOrDuplicate => {
                    preflight.non_contiguous_or_duplicate += 1;
                }
                AuditVerdict::HashMismatch => preflight.hash_mismatch += 1,
            }
            push_sample(&mut preflight.samples, verdict, document.id, sample_size);
            preflight.scanned += 1;
            after_id = Some(document.id);
            if i64::try_from(preflight.scanned).unwrap_or(i64::MAX) >= max_documents {
                break;
            }
        }
        if i64::try_from(preflight.scanned).unwrap_or(i64::MAX) >= max_documents {
            if i64::try_from(page.len()).unwrap_or(0) >= limit {
                let next = list_missing_file_library_page(pool, after_id, 1).await?;
                preflight.truncated = !next.is_empty();
            }
            break;
        }
        if i64::try_from(page.len()).unwrap_or(0) < limit {
            break;
        }
    }
    Ok(preflight)
}
