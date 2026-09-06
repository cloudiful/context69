//! Per-document `file_library` backfill apply (issue 139, phase 4).
//!
//! One transaction per document: `FOR UPDATE` lock, ordered-chunk
//! reconstruction, application-side SHA-256 verification, idempotent
//! `ON CONFLICT DO NOTHING` insert, and pre-commit verification. Never
//! updates documents/chunks/tasks, retries jobs, migrates, or overwrites
//! existing versions.

use anyhow::{Context, Result};
use chrono::Utc;
use sqlx::PgPool;

use super::types::{
    BackfillApplySummary, BackfillChunkRow, BackfillErrorDoc, BackfillSkippedDoc,
    LockedFileLibraryDocument,
};
use crate::domain::SourceRecord;
use crate::normalize::normalize_record;

enum SingleOutcome {
    Inserted,
    AlreadyFixed,
    Skipped(String),
}

/// Recompute the application hash for the locked fields and
/// reconstructed body. Mirrors `refresh_metadata_for_file`: ordered
/// chunks joined with `"\n"` fed through `normalize_record` with the
/// current document fields. `published_at` is carried for snapshot
/// fidelity even though the hash covers
/// title/summary/body/source_uri/metadata_json.
fn recomputed_backfill_hash(locked: &LockedFileLibraryDocument, raw_body: &str) -> String {
    normalize_record(SourceRecord {
        external_id: format!("backfill-{}", locked.id),
        title: locked.title.clone(),
        body_text: raw_body.to_string(),
        source_uri: locked.source_uri.clone(),
        summary: locked.summary.clone(),
        published_at: locked.published_at,
        updated_at: Utc::now(),
        metadata_json: locked.metadata_json.clone(),
    })
    .record_hash
}

/// Backfill one document in its own transaction. Locks the row, re-reads
/// current fields, validates shape and hash, inserts the complete
/// snapshot idempotently, verifies before commit, and rolls back on any
/// skip/error without touching documents/chunks/tasks.
async fn backfill_one_document(pool: &PgPool, document_id: i64) -> Result<SingleOutcome> {
    let mut tx = pool.begin().await.context("begin backfill transaction")?;
    let locked = sqlx::query_file_as!(
        LockedFileLibraryDocument,
        "src/sql/db/documents/get_file_library_document_version_for_update.sql",
        document_id
    )
    .fetch_optional(&mut *tx)
    .await
    .context("lock file_library document for backfill")?;
    let Some(locked) = locked else {
        tx.rollback().await.ok();
        return Ok(SingleOutcome::Skipped(
            "not_file_library_or_missing".to_string(),
        ));
    };

    let already: bool = sqlx::query_file_scalar!(
        "src/sql/db/documents/verify_document_version.sql",
        locked.id,
        locked.record_hash
    )
    .fetch_one(&mut *tx)
    .await
    .context("verify existing version for backfill")?
    .unwrap_or(false);
    if already {
        tx.rollback().await.ok();
        return Ok(SingleOutcome::AlreadyFixed);
    }

    let chunk_rows = sqlx::query_file_as!(
        BackfillChunkRow,
        "src/sql/db/documents/get_document_chunks.sql",
        locked.id
    )
    .fetch_all(&mut *tx)
    .await
    .context("load ordered chunks inside backfill transaction")?;
    if chunk_rows.is_empty() {
        tx.rollback().await.ok();
        return Ok(SingleOutcome::Skipped("zero_chunks".to_string()));
    }
    // Contiguity is checked on sorted indexes so gaps, non-zero starts,
    // and duplicates all fail before any hash comparison.
    let mut indexes: Vec<i32> = chunk_rows.iter().map(|row| row.chunk_index).collect();
    indexes.sort_unstable();
    for (position, index) in indexes.iter().enumerate() {
        let Ok(expected) = i32::try_from(position) else {
            tx.rollback().await.ok();
            return Ok(SingleOutcome::Skipped(
                "non_contiguous_or_duplicate".to_string(),
            ));
        };
        if *index != expected {
            tx.rollback().await.ok();
            return Ok(SingleOutcome::Skipped(
                "non_contiguous_or_duplicate".to_string(),
            ));
        }
    }
    // `get_document_chunks.sql` already orders by chunk_index, so the join
    // below reconstructs the deterministic ordered body.
    let raw_body = chunk_rows
        .iter()
        .map(|row| row.chunk_text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    if crate::normalize::normalize_body(&raw_body)
        .trim()
        .is_empty()
    {
        tx.rollback().await.ok();
        return Ok(SingleOutcome::Skipped("blank_body".to_string()));
    }
    if recomputed_backfill_hash(&locked, &raw_body) != locked.record_hash {
        tx.rollback().await.ok();
        return Ok(SingleOutcome::Skipped("hash_mismatch".to_string()));
    }

    sqlx::query_file!(
        "src/sql/db/documents/insert_document_version.sql",
        locked.id,
        locked.record_hash,
        locked.title,
        locked.summary,
        raw_body,
        locked.source_uri,
        locked.published_at,
        locked.metadata_json
    )
    .execute(&mut *tx)
    .await
    .context("insert backfill version snapshot")?;

    let verified: bool = sqlx::query_file_scalar!(
        "src/sql/db/documents/verify_document_version.sql",
        locked.id,
        locked.record_hash
    )
    .fetch_one(&mut *tx)
    .await
    .context("verify backfill version before commit")?
    .unwrap_or(false);
    if !verified {
        tx.rollback().await.ok();
        anyhow::bail!(
            "backfill version missing after insert for document {}",
            locked.id
        );
    }
    tx.commit().await.context("commit backfill transaction")?;
    Ok(SingleOutcome::Inserted)
}

/// Apply the backfill over a preflight-approved id list. One transaction
/// per document; each transaction rolls back on skip/error. Resumable:
/// rerunning with the same ids reports already-fixed rows without
/// overwriting them.
pub async fn apply_file_library_backfill(
    pool: &PgPool,
    eligible_ids: &[i64],
) -> Result<BackfillApplySummary> {
    let mut ordered = eligible_ids.to_vec();
    ordered.sort_unstable();
    ordered.dedup();
    let mut summary = BackfillApplySummary {
        scanned: ordered.len(),
        inserted: 0,
        already_fixed: 0,
        skipped: 0,
        errored: 0,
        inserted_ids: Vec::new(),
        already_fixed_ids: Vec::new(),
        skipped_docs: Vec::new(),
        error_docs: Vec::new(),
    };
    for document_id in ordered {
        match backfill_one_document(pool, document_id).await {
            Ok(SingleOutcome::Inserted) => {
                summary.inserted += 1;
                summary.inserted_ids.push(document_id);
            }
            Ok(SingleOutcome::AlreadyFixed) => {
                summary.already_fixed += 1;
                summary.already_fixed_ids.push(document_id);
            }
            Ok(SingleOutcome::Skipped(reason)) => {
                summary.skipped += 1;
                summary.skipped_docs.push(BackfillSkippedDoc {
                    id: document_id,
                    reason,
                });
            }
            Err(error) => {
                summary.errored += 1;
                summary.error_docs.push(BackfillErrorDoc {
                    id: document_id,
                    // Operational message only; bodies are never included.
                    error: format!("{error:#}"),
                });
            }
        }
    }
    Ok(summary)
}
