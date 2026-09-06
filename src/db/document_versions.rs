//! Version-snapshot helpers for library business-fields updates (issue 139).
//!
//! `update_library_document_business_fields` changes `documents.record_hash`
//! without going through `upsert_document`, so it must persist a matching
//! `document_versions` row in the same transaction. Same-hash publishes stay
//! a no-op for versions; changed hashes reconstruct the complete body from
//! ordered chunks and fail closed when no valid body exists.

use anyhow::{Context, Result};
use sqlx::FromRow;

use crate::domain::ChunkPayload;

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
