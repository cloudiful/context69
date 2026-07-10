use std::collections::HashMap;

use anyhow::{Context, Result};
use uuid::Uuid;

use super::{
    ChunkRow, Database, DocumentRow, ExistingDocumentRow, KeywordSearchHitRow, ReindexChunkRow,
    SearchHitRow, UpsertedDocument, is_library_file, keyword_terms, library_file_id, library_path,
    library_section_label, search_hit_from_keyword_row,
};
use crate::contracts::{
    DocumentChunkResponse, DocumentResponse, SearchHit, SearchRequest, Visibility,
};
use crate::domain::{AccessScope, ChunkPayload, DocumentChunk};
use crate::normalize::is_meaningful_text;

impl Database {
    pub async fn upsert_document(&self, payload: &ChunkPayload) -> Result<UpsertedDocument> {
        let mut tx = self.pool.begin().await?;
        let existing = sqlx::query_file_as!(
            ExistingDocumentRow,
            "src/sql/db/documents/get_existing_document.sql",
            payload.group_id,
            payload.source_key,
            payload.external_id
        )
        .fetch_optional(&mut *tx)
        .await?;

        let (document_id, changed) = match existing {
            Some(existing) if existing.record_hash == payload.record_hash => {
                sqlx::query_file!(
                    "src/sql/db/documents/update_document_unchanged.sql",
                    payload.group_id,
                    payload.source_key,
                    payload.title,
                    payload.summary,
                    payload.source_uri,
                    payload.published_at,
                    payload.updated_at_source,
                    payload.metadata_json,
                    payload.external_id
                )
                .execute(&mut *tx)
                .await?;
                (existing.id, false)
            }
            Some(existing) => {
                sqlx::query_file!(
                    "src/sql/db/documents/update_document_changed.sql",
                    payload.group_id,
                    payload.source_key,
                    payload.title,
                    payload.summary,
                    payload.source_uri,
                    payload.published_at,
                    payload.updated_at_source,
                    payload.metadata_json,
                    payload.record_hash,
                    payload.external_id
                )
                .execute(&mut *tx)
                .await?;
                (existing.id, true)
            }
            None => {
                let id = sqlx::query_file_scalar!(
                    "src/sql/db/documents/insert_document.sql",
                    payload.group_id,
                    payload.visibility.as_str(),
                    payload.source_key,
                    payload.external_id,
                    payload.title,
                    payload.summary,
                    payload.source_uri,
                    payload.published_at,
                    payload.updated_at_source,
                    payload.metadata_json,
                    payload.record_hash
                )
                .fetch_one(&mut *tx)
                .await?;
                (id, true)
            }
        };

        if changed {
            sqlx::query_file!(
                "src/sql/db/documents/insert_document_version.sql",
                document_id,
                payload.record_hash,
                payload.title,
                payload.summary,
                payload.chunk_text,
                payload.source_uri,
                payload.published_at,
                payload.metadata_json
            )
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(UpsertedDocument {
            document_id,
            changed,
        })
    }

    pub async fn replace_document_chunks(
        &self,
        document_id: i64,
        record_hash: &str,
        chunks: &[DocumentChunk],
    ) -> Result<Vec<Uuid>> {
        let mut tx = self.pool.begin().await?;
        sqlx::query_file!(
            "src/sql/db/documents/delete_document_chunks.sql",
            document_id
        )
        .execute(&mut *tx)
        .await?;

        for chunk in chunks {
            sqlx::query_file!(
                "src/sql/db/documents/insert_document_chunk.sql",
                chunk.id,
                document_id,
                chunk.chunk_index,
                chunk.text,
                record_hash
            )
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(chunks.iter().map(|chunk| chunk.id).collect())
    }

    pub async fn list_chunk_ids_for_document(&self, document_id: i64) -> Result<Vec<Uuid>> {
        Ok(sqlx::query_file_scalar!(
            "src/sql/db/documents/list_chunk_ids_for_document.sql",
            document_id
        )
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn fetch_search_hits_by_chunk_ids(
        &self,
        chunk_ids: &[Uuid],
        scope: &AccessScope,
    ) -> Result<HashMap<Uuid, SearchHit>> {
        let rows = sqlx::query_file_as!(
            SearchHitRow,
            "src/sql/db/documents/fetch_search_hits_by_chunk_ids.sql",
            chunk_ids,
            &scope.private_group_ids,
            scope.group_path.as_deref()
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| {
                (
                    row.chunk_id,
                    SearchHit {
                        chunk_id: row.chunk_id,
                        document_id: row.document_id,
                        group_key: row.group_key,
                        group_path: row.group_path,
                        visibility: row.visibility.parse().unwrap_or(Visibility::Private),
                        source_key: row.source_key,
                        external_id: row.external_id,
                        title: row.title,
                        summary: row.summary.filter(|value| is_meaningful_text(value)),
                        source_uri: row.source_uri,
                        published_at: row.published_at,
                        chunk_index: row.chunk_index,
                        chunk_text: row.chunk_text,
                        score: 0.0,
                        vector_score: None,
                        keyword_score: None,
                        rerank_score: None,
                        match_reason: None,
                        library_file_id: library_file_id(&row.metadata_json),
                        library_section_label: library_section_label(&row.metadata_json),
                        library_path: library_path(&row.metadata_json),
                        is_library_file: is_library_file(&row.metadata_json),
                        metadata_json: row.metadata_json,
                    },
                )
            })
            .collect())
    }

    pub async fn keyword_search(
        &self,
        request: &SearchRequest,
        scope: &AccessScope,
        limit: usize,
    ) -> Result<Vec<SearchHit>> {
        let normalized_query = request.query.trim().to_lowercase();
        if normalized_query.is_empty() {
            return Ok(Vec::new());
        }
        let phrase_pattern = format!("%{normalized_query}%");
        let terms = keyword_terms(&normalized_query);
        let keyword_limit = i64::try_from(limit).context("keyword search limit is too large")?;

        let rows = sqlx::query_file_as!(
            KeywordSearchHitRow,
            "src/sql/db/documents/keyword_search.sql",
            normalized_query,
            phrase_pattern,
            &terms,
            request.source_key,
            request.group_path.as_deref(),
            request.published_after,
            request.published_before,
            &scope.private_group_ids,
            keyword_limit
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(search_hit_from_keyword_row).collect())
    }

    pub async fn list_chunk_payloads_for_reindex(&self) -> Result<Vec<ChunkPayload>> {
        let rows = sqlx::query_file_as!(
            ReindexChunkRow,
            "src/sql/db/documents/list_chunk_payloads_for_reindex.sql"
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| ChunkPayload {
                chunk_id: row.chunk_id,
                document_id: row.document_id,
                group_id: row.group_id,
                group_key: row.group_key,
                group_path: row.group_path,
                visibility: row.visibility.parse().unwrap_or(Visibility::Private),
                source_key: row.source_key,
                external_id: row.external_id,
                title: row.title,
                summary: row.summary,
                source_uri: row.source_uri,
                published_at: row.published_at,
                updated_at_source: row.updated_at_source,
                record_hash: row.record_hash,
                chunk_index: row.chunk_index,
                chunk_text: row.chunk_text,
                metadata_json: row.metadata_json,
            })
            .collect())
    }

    pub async fn get_document(
        &self,
        document_id: i64,
        scope: &AccessScope,
    ) -> Result<Option<DocumentResponse>> {
        let document = sqlx::query_file_as!(
            DocumentRow,
            "src/sql/db/documents/get_document.sql",
            document_id,
            &scope.private_group_ids,
            scope.group_path.as_deref()
        )
        .fetch_optional(&self.pool)
        .await?;

        let Some(document) = document else {
            return Ok(None);
        };

        let chunks = sqlx::query_file_as!(
            ChunkRow,
            "src/sql/db/documents/get_document_chunks.sql",
            document_id
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(Some(DocumentResponse {
            document_id: document.id,
            group_key: document.group_key,
            group_path: document.group_path,
            visibility: document.visibility.parse().unwrap_or(Visibility::Private),
            source_key: document.source_key,
            external_id: document.external_id,
            title: document.title,
            summary: document.summary.filter(|value| is_meaningful_text(value)),
            source_uri: document.source_uri,
            published_at: document.published_at,
            updated_at: document.updated_at_source,
            record_hash: document.record_hash,
            library_file_id: library_file_id(&document.metadata_json),
            library_section_label: library_section_label(&document.metadata_json),
            library_path: library_path(&document.metadata_json),
            is_library_file: is_library_file(&document.metadata_json),
            metadata_json: document.metadata_json,
            chunks: chunks
                .into_iter()
                .filter(|chunk| is_meaningful_text(&chunk.chunk_text))
                .map(|chunk| DocumentChunkResponse {
                    chunk_id: chunk.id,
                    chunk_index: chunk.chunk_index,
                    text: chunk.chunk_text,
                })
                .collect(),
        }))
    }
}
