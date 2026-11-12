//! Fresh document-rechunk rebuild staging (issue #27 P2).
//!
//! The rebuild records, per physical target collection, the fresh original
//! chunks produced from canonical document bodies and the current translated
//! chunks replayed beside them. The staged ids are stable across a resume, so a
//! repeated page is idempotent and activation installs exactly the ids that
//! were embedded into the target.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgConnection;
use uuid::Uuid;

use super::Database;
use crate::chunk_payload::{
    ChunkRef, PayloadDocument, PayloadGroup, PayloadLocale, original_chunk, translated_chunk,
    visibility_or_private,
};
use crate::domain::{ChunkPayload, DocumentChunk};

/// One current document joined with its canonical version body.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct RebuildDocumentRow {
    pub document_id: i64,
    pub group_id: i64,
    pub group_key: String,
    pub group_path: String,
    pub visibility: String,
    pub source_key: String,
    pub external_id: String,
    pub title: String,
    pub summary: Option<String>,
    pub source_uri: String,
    pub published_at: Option<DateTime<Utc>>,
    pub updated_at_source: DateTime<Utc>,
    pub record_hash: String,
    pub metadata_json: Value,
    pub body_text: String,
}

/// One staged fresh original chunk with its document and group fields.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct StagedRebuildChunkRow {
    pub chunk_id: Uuid,
    pub document_id: i64,
    pub chunk_index: i32,
    pub chunk_text: String,
    pub record_hash: String,
    pub group_id: i64,
    pub group_key: String,
    pub group_path: String,
    pub visibility: String,
    pub source_key: String,
    pub external_id: String,
    pub title: String,
    pub summary: Option<String>,
    pub source_uri: String,
    pub published_at: Option<DateTime<Utc>>,
    pub updated_at_source: DateTime<Utc>,
    pub metadata_json: Value,
}

/// One staged translated chunk with its version, document and group fields.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct StagedRebuildTranslationRow {
    pub chunk_id: Uuid,
    pub document_id: i64,
    pub chunk_index: i32,
    pub chunk_text: String,
    pub record_hash: String,
    pub translation_id: Uuid,
    pub target_locale: String,
    pub source_locale: Option<String>,
    pub provider_key: String,
    pub translated_title: String,
    pub translated_summary: Option<String>,
    pub group_id: i64,
    pub group_key: String,
    pub group_path: String,
    pub visibility: String,
    pub source_key: String,
    pub external_id: String,
    pub source_uri: String,
    pub published_at: Option<DateTime<Utc>>,
    pub updated_at_source: DateTime<Utc>,
    pub metadata_json: Value,
}

/// The exact staged totals of one target.
#[derive(Debug, Clone, Copy, sqlx::FromRow)]
pub struct RebuildStagingCounts {
    #[sqlx(rename = "original")]
    pub original: i64,
    #[sqlx(rename = "translated")]
    pub translated: i64,
}

/// The fail-closed verification of the staged snapshot.
#[derive(Debug, Clone, Copy, sqlx::FromRow)]
pub struct RebuildStagingVerification {
    #[sqlx(rename = "document_hash_mismatch")]
    pub document_hash_mismatch: i64,
    #[sqlx(rename = "chunk_count_mismatch")]
    pub chunk_count_mismatch: i64,
    #[sqlx(rename = "chunk_hash_mismatch")]
    pub chunk_hash_mismatch: i64,
    #[sqlx(rename = "staged_documents")]
    pub staged_documents: i64,
    #[sqlx(rename = "document_membership_mismatch")]
    pub document_membership_mismatch: i64,
}

/// One file-group of fresh original chunks.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct RebuildOriginalCountRow {
    pub file_id: Option<Uuid>,
    #[sqlx(rename = "count")]
    pub count: i64,
}

/// One staged original chunk key of the immutable installation keyset.
#[derive(Debug, Clone, Copy, sqlx::FromRow)]
pub struct RebuildInstallChunkKey {
    pub document_id: i64,
    pub chunk_index: i32,
    pub chunk_id: Uuid,
}

/// The fail-closed proof that the live chunk table is exactly the staged
/// original snapshot.
#[derive(Debug, Clone, Copy, sqlx::FromRow)]
pub struct RebuildInstallVerification {
    #[sqlx(rename = "live_chunks")]
    pub live_chunks: i64,
    #[sqlx(rename = "staged_chunks")]
    pub staged_chunks: i64,
    #[sqlx(rename = "mismatch")]
    pub mismatch: i64,
}

/// The projected registry usage after rebuild delta entries are written.
#[derive(Debug, Clone, Copy, sqlx::FromRow)]
pub struct RebuildProjectedUsage {
    #[sqlx(rename = "total")]
    pub total: i64,
    #[sqlx(rename = "max_file")]
    pub max_file: i64,
    #[sqlx(rename = "max_run")]
    pub max_run: i64,
}

/// The payload one staged original chunk yields.
pub fn staged_original_payload(row: &StagedRebuildChunkRow) -> ChunkPayload {
    original_chunk(
        &PayloadGroup {
            group_id: row.group_id,
            group_key: &row.group_key,
            group_path: &row.group_path,
            visibility: visibility_or_private(&row.visibility),
        },
        &PayloadDocument {
            source_key: &row.source_key,
            external_id: &row.external_id,
            title: &row.title,
            summary: row.summary.as_deref(),
            source_uri: &row.source_uri,
            published_at: row.published_at,
            updated_at_source: row.updated_at_source,
            record_hash: &row.record_hash,
            metadata_json: &row.metadata_json,
        },
        ChunkRef {
            chunk_id: row.chunk_id,
            document_id: row.document_id,
            chunk_index: row.chunk_index,
            chunk_text: &row.chunk_text,
        },
    )
}

/// The payload one staged translated chunk yields.
pub fn staged_translation_payload(row: &StagedRebuildTranslationRow) -> ChunkPayload {
    translated_chunk(
        &PayloadGroup {
            group_id: row.group_id,
            group_key: &row.group_key,
            group_path: &row.group_path,
            visibility: visibility_or_private(&row.visibility),
        },
        &PayloadDocument {
            source_key: &row.source_key,
            external_id: &row.external_id,
            title: &row.translated_title,
            summary: row.translated_summary.as_deref(),
            source_uri: &row.source_uri,
            published_at: row.published_at,
            updated_at_source: row.updated_at_source,
            record_hash: &row.record_hash,
            metadata_json: &row.metadata_json,
        },
        ChunkRef {
            chunk_id: row.chunk_id,
            document_id: row.document_id,
            chunk_index: row.chunk_index,
            chunk_text: &row.chunk_text,
        },
        PayloadLocale {
            content_locale: &row.target_locale,
            source_locale: row.source_locale.as_deref(),
            translation_provider: Some(&row.provider_key),
        },
    )
}

impl Database {
    pub async fn list_rebuild_documents_page(
        &self,
        after_document_id: Option<i64>,
        limit: i64,
    ) -> Result<Vec<RebuildDocumentRow>> {
        Ok(sqlx::query_file_as!(
            RebuildDocumentRow,
            "src/sql/db/documents/list_rebuild_documents_page.sql",
            after_document_id,
            limit,
        )
        .fetch_all(&self.pool)
        .await
        .context("list rebuild documents page")?)
    }

    pub async fn list_rebuild_missing_documents_page(
        &self,
        after_document_id: Option<i64>,
        limit: i64,
    ) -> Result<Vec<i64>> {
        Ok(sqlx::query_file_scalar!(
            "src/sql/db/documents/list_rebuild_missing_documents_page.sql",
            after_document_id,
            limit,
        )
        .fetch_all(&self.pool)
        .await
        .context("list rebuild documents without a canonical body")?)
    }

    /// Stage one document's fresh chunks, its progress and its current
    /// translations in one transaction.
    ///
    /// The staging cursor advances in the same transaction under the owner
    /// guard, so a cancelled or superseded attempt commits neither the staged
    /// rows nor the progress. Returns whether the owner still held the
    /// barrier.
    pub async fn stage_rebuild_document(
        &self,
        target_collection: &str,
        document: &RebuildDocumentRow,
        chunks: &[DocumentChunk],
        owner_item_id: Uuid,
        owner_token: Uuid,
        fingerprint: &str,
    ) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        sqlx::query_file!(
            "src/sql/db/documents/delete_rebuild_staging_chunks_for_document.sql",
            target_collection,
            document.document_id,
        )
        .execute(&mut *tx)
        .await?;
        if !chunks.is_empty() {
            let ids: Vec<Uuid> = chunks.iter().map(|chunk| chunk.id).collect();
            let indexes: Vec<i32> = chunks.iter().map(|chunk| chunk.chunk_index).collect();
            let texts: Vec<String> = chunks.iter().map(|chunk| chunk.text.clone()).collect();
            sqlx::query_file!(
                "src/sql/db/documents/insert_rebuild_staging_chunks.sql",
                target_collection,
                document.document_id,
                &document.record_hash,
                &ids,
                &indexes,
                &texts,
            )
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query_file!(
            "src/sql/db/documents/upsert_rebuild_staging_document.sql",
            target_collection,
            document.document_id,
            &document.record_hash,
            i32::try_from(chunks.len()).context("staged chunk count within i32")?,
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query_file!(
            "src/sql/db/translations/copy_current_translation_chunks_for_document.sql",
            target_collection,
            document.document_id,
        )
        .execute(&mut *tx)
        .await?;
        let owned = self
            .advance_rebuild_staging_in(
                &mut tx,
                owner_item_id,
                owner_token,
                fingerprint,
                target_collection,
                document.document_id,
            )
            .await?;
        if !owned {
            tx.rollback().await?;
            return Ok(false);
        }
        tx.commit().await?;
        Ok(true)
    }

    pub async fn count_rebuild_staging_chunks(
        &self,
        target_collection: &str,
    ) -> Result<RebuildStagingCounts> {
        Ok(sqlx::query_file_as!(
            RebuildStagingCounts,
            "src/sql/db/documents/count_rebuild_staging_chunks.sql",
            target_collection,
        )
        .fetch_one(&self.pool)
        .await
        .context("count rebuild staging chunks")?)
    }

    /// The staged and total document counts of one running rebuild target.
    ///
    /// Staged documents are cut and staged, not necessarily embedded, and the
    /// total is the canonical document set the rebuild selects.
    pub async fn read_rebuild_document_progress(
        &self,
        target_collection: &str,
    ) -> Result<(i64, i64)> {
        let row = sqlx::query_file!(
            "src/sql/db/documents/count_rebuild_progress_documents.sql",
            target_collection,
        )
        .fetch_one(&self.pool)
        .await
        .context("count rebuild progress documents")?;
        Ok((row.staged, row.total))
    }

    pub async fn list_rebuild_staging_chunks_page(
        &self,
        target_collection: &str,
        after_document_id: Option<i64>,
        after_chunk_index: Option<i32>,
        limit: i64,
    ) -> Result<Vec<StagedRebuildChunkRow>> {
        Ok(sqlx::query_file_as!(
            StagedRebuildChunkRow,
            "src/sql/db/documents/list_rebuild_staging_chunks_page.sql",
            target_collection,
            after_document_id,
            after_chunk_index.unwrap_or(-1),
            limit,
        )
        .fetch_all(&self.pool)
        .await
        .context("list staged rebuild chunks page")?)
    }

    pub async fn list_translation_rebuild_staging_page(
        &self,
        target_collection: &str,
        after_document_id: Option<i64>,
        after_translation_id: Option<Uuid>,
        after_chunk_index: Option<i32>,
        limit: i64,
    ) -> Result<Vec<StagedRebuildTranslationRow>> {
        Ok(sqlx::query_file_as!(
            StagedRebuildTranslationRow,
            "src/sql/db/translations/list_translation_rebuild_staging_page.sql",
            target_collection,
            after_document_id,
            after_translation_id,
            after_chunk_index.unwrap_or(-1),
            limit,
        )
        .fetch_all(&self.pool)
        .await
        .context("list staged translation chunks page")?)
    }

    pub async fn verify_rebuild_staging(
        &self,
        target_collection: &str,
    ) -> Result<RebuildStagingVerification> {
        let mut connection = self.pool.acquire().await?;
        self.verify_rebuild_staging_in(&mut connection, target_collection)
            .await
    }

    pub async fn verify_rebuild_staging_in(
        &self,
        connection: &mut PgConnection,
        target_collection: &str,
    ) -> Result<RebuildStagingVerification> {
        Ok(sqlx::query_file_as!(
            RebuildStagingVerification,
            "src/sql/db/documents/verify_rebuild_staging.sql",
            target_collection,
        )
        .fetch_one(connection)
        .await
        .context("verify the staged rebuild snapshot")?)
    }

    pub async fn read_projected_registry_usage_in(
        &self,
        connection: &mut PgConnection,
        target_collection: &str,
    ) -> Result<RebuildProjectedUsage> {
        Ok(sqlx::query_file_as!(
            RebuildProjectedUsage,
            "src/sql/db/search_registry/read_projected_usage.sql",
            target_collection,
        )
        .fetch_one(connection)
        .await
        .context("read the projected registry usage")?)
    }

    /// Bound how long the one-time reset waits for the chunk table lock.
    pub async fn set_rebuild_install_lock_timeout_in(
        &self,
        connection: &mut PgConnection,
        timeout: &str,
    ) -> Result<()> {
        sqlx::query_file_scalar!("src/sql/db/documents/set_local_lock_timeout.sql", timeout)
            .fetch_one(&mut *connection)
            .await
            .context("set the reset lock timeout")?;
        Ok(())
    }

    /// Drop only the derived original chunk rows; canonical data is untouched.
    pub async fn truncate_document_chunks_in(&self, connection: &mut PgConnection) -> Result<()> {
        sqlx::query_file!("src/sql/db/documents/truncate_document_chunks.sql")
            .execute(&mut *connection)
            .await
            .context("truncate the derived original chunks")?;
        Ok(())
    }

    /// One immutable keyset page of staged original chunks to install.
    pub async fn list_rebuild_install_chunk_page_in(
        &self,
        connection: &mut PgConnection,
        target_collection: &str,
        after: Option<&super::RebuildCursor>,
        limit: i64,
    ) -> Result<Vec<RebuildInstallChunkKey>> {
        sqlx::query_file_as!(
            RebuildInstallChunkKey,
            "src/sql/db/documents/list_rebuild_install_chunk_page.sql",
            target_collection,
            after.map(|cursor| cursor.document_id),
            after.map(|cursor| cursor.chunk_index).unwrap_or(-1),
            limit,
        )
        .fetch_all(connection)
        .await
        .context("list the rebuild install chunk page")
    }

    /// Install exactly one keyset page of staged original chunks.
    pub async fn insert_rebuild_install_chunk_page_in(
        &self,
        connection: &mut PgConnection,
        target_collection: &str,
        after: Option<&super::RebuildCursor>,
        up_to: &super::RebuildCursor,
    ) -> Result<()> {
        sqlx::query_file!(
            "src/sql/db/documents/insert_rebuild_install_chunk_page.sql",
            target_collection,
            after.map(|cursor| cursor.document_id),
            after.map(|cursor| cursor.chunk_index).unwrap_or(-1),
            up_to.document_id,
            up_to.chunk_index,
        )
        .execute(&mut *connection)
        .await
        .context("install the rebuild chunk page")?;
        Ok(())
    }

    /// Count the rows of one installed page that do not match their source.
    pub async fn prove_rebuild_install_chunk_page_in(
        &self,
        connection: &mut PgConnection,
        target_collection: &str,
        after: Option<&super::RebuildCursor>,
        up_to: &super::RebuildCursor,
    ) -> Result<i64> {
        Ok(sqlx::query_file!(
            "src/sql/db/documents/prove_rebuild_install_chunk_page.sql",
            target_collection,
            after.map(|cursor| cursor.document_id),
            after.map(|cursor| cursor.chunk_index).unwrap_or(-1),
            up_to.document_id,
            up_to.chunk_index,
        )
        .fetch_one(connection)
        .await
        .context("prove the installed rebuild chunk page")?
        .mismatch)
    }

    /// Per-file original counts of one installed page only.
    pub async fn rebuild_install_page_file_counts_in(
        &self,
        connection: &mut PgConnection,
        target_collection: &str,
        after: Option<&super::RebuildCursor>,
        up_to: &super::RebuildCursor,
    ) -> Result<Vec<RebuildOriginalCountRow>> {
        sqlx::query_file_as!(
            RebuildOriginalCountRow,
            "src/sql/db/documents/rebuild_install_page_file_counts.sql",
            target_collection,
            after.map(|cursor| cursor.document_id),
            after.map(|cursor| cursor.chunk_index).unwrap_or(-1),
            up_to.document_id,
            up_to.chunk_index,
        )
        .fetch_all(connection)
        .await
        .context("count the installed rebuild page by file")
    }

    /// The fail-closed proof that the live chunk table is the staged snapshot.
    pub async fn verify_rebuild_install_in(
        &self,
        connection: &mut PgConnection,
        target_collection: &str,
    ) -> Result<RebuildInstallVerification> {
        sqlx::query_file_as!(
            RebuildInstallVerification,
            "src/sql/db/documents/verify_rebuild_install.sql",
            target_collection,
        )
        .fetch_one(connection)
        .await
        .context("verify the installed rebuild chunks")
    }
}
