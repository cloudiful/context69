use std::collections::HashMap;

use anyhow::{Context, Result};
use uuid::Uuid;

use super::{
    ChunkRow, Database, DocumentChunkRow, DocumentKeyLookupRow, DocumentRow, ExistingDocumentRow,
    KeywordSearchHitRow, ReindexChunkRow, SearchHitRow, TranslationChunkBatchRow,
    TranslationStatusBatchRow, TranslationVersionBatchRow, UpsertedDocument, is_library_file,
    keyword_terms, library_file_id, library_path, library_section_label,
    search_hit_from_keyword_row, translation_status,
};
use crate::contracts::{
    DocumentChunkResponse, DocumentResponse, SearchHit, SearchRequest, TranslationStatus,
    Visibility,
};
use crate::domain::{AccessScope, ChunkPayload, DocumentChunk};
use crate::normalize::is_meaningful_text;

impl Database {
    pub async fn find_document_id_by_key(
        &self,
        group_id: i64,
        source_key: &str,
        external_id: &str,
    ) -> Result<Option<i64>> {
        Ok(sqlx::query_file_scalar!(
            "src/sql/db/documents/find_id_by_key.sql",
            group_id,
            source_key,
            external_id
        )
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn list_document_ids_by_keys(
        &self,
        group_id: i64,
        source_keys: &[String],
        external_ids: &[String],
    ) -> Result<Vec<Option<i64>>> {
        if source_keys.is_empty() {
            return Ok(Vec::new());
        }
        if source_keys.len() != external_ids.len() {
            return Err(anyhow::anyhow!(
                "document key arrays must have the same length"
            ));
        }

        let rows = sqlx::query_file_as!(
            DocumentKeyLookupRow,
            "src/sql/db/documents/get_document_ids_by_keys.sql",
            group_id,
            source_keys,
            external_ids
        )
        .fetch_all(&self.pool)
        .await?;

        let mut result = vec![None; source_keys.len()];
        for row in rows {
            let position = usize::try_from(row.ordinal.saturating_sub(1))
                .context("document key ordinal is out of range")?;
            if let Some(slot) = result.get_mut(position) {
                *slot = row.document_id;
            }
        }
        Ok(result)
    }

    pub async fn get_documents_localized(
        &self,
        document_ids: &[i64],
        locale: Option<&str>,
        scope: &AccessScope,
    ) -> Result<HashMap<i64, DocumentResponse>> {
        if document_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let ids = document_ids.to_vec();
        let documents = sqlx::query_file_as!(
            DocumentRow,
            "src/sql/db/documents/get_documents_by_ids.sql",
            &ids,
            &scope.private_group_ids,
            scope.group_path.as_deref()
        )
        .fetch_all(&self.pool)
        .await?;
        let chunks = sqlx::query_file_as!(
            DocumentChunkRow,
            "src/sql/db/documents/get_document_chunks_by_ids.sql",
            &ids
        )
        .fetch_all(&self.pool)
        .await?;

        let mut chunks_by_document = HashMap::<i64, Vec<DocumentChunkResponse>>::new();
        for chunk in chunks {
            if is_meaningful_text(&chunk.chunk_text) {
                chunks_by_document
                    .entry(chunk.document_id)
                    .or_default()
                    .push(DocumentChunkResponse {
                        chunk_id: chunk.id,
                        chunk_index: chunk.chunk_index,
                        text: chunk.chunk_text,
                    });
            }
        }

        let mut result = HashMap::with_capacity(documents.len());
        for document in documents {
            let document_id = document.id;
            let chunks = chunks_by_document.remove(&document_id).unwrap_or_default();
            result.insert(
                document_id,
                document_response_from_parts(document, chunks, None, None, false),
            );
        }

        let Some(locale) = locale else {
            for document in result.values_mut() {
                document.content_locale = Some("original".to_string());
            }
            return Ok(result);
        };

        let versions = sqlx::query_file_as!(
            TranslationVersionBatchRow,
            "src/sql/db/translations/get_versions_by_document_ids.sql",
            &ids,
            locale
        )
        .fetch_all(&self.pool)
        .await?;
        let version_ids = versions
            .iter()
            .map(|version| version.id)
            .collect::<Vec<_>>();
        let translation_chunks = if version_ids.is_empty() {
            Vec::new()
        } else {
            sqlx::query_file_as!(
                TranslationChunkBatchRow,
                "src/sql/db/translations/get_chunks_by_version_ids.sql",
                &version_ids
            )
            .fetch_all(&self.pool)
            .await?
        };
        let statuses = sqlx::query_file_as!(
            TranslationStatusBatchRow,
            "src/sql/db/translations/get_status_by_document_ids.sql",
            &ids,
            locale
        )
        .fetch_all(&self.pool)
        .await?;

        let versions_by_document = versions
            .into_iter()
            .map(|version| (version.document_id, version))
            .collect::<HashMap<_, _>>();
        let mut chunks_by_version = HashMap::<Uuid, Vec<DocumentChunkResponse>>::new();
        for chunk in translation_chunks {
            chunks_by_version
                .entry(chunk.translation_id)
                .or_default()
                .push(DocumentChunkResponse {
                    chunk_id: chunk.id,
                    chunk_index: chunk.chunk_index,
                    text: chunk.chunk_text,
                });
        }
        let statuses_by_document = statuses
            .into_iter()
            .map(|status| (status.document_id, status))
            .collect::<HashMap<_, _>>();

        for (document_id, document) in &mut result {
            if let Some(version) = versions_by_document.get(document_id) {
                document.title = version.translated_title.clone();
                document.summary = version.translated_summary.clone();
                document.chunks = chunks_by_version
                    .get(&version.id)
                    .cloned()
                    .unwrap_or_default();
                document.requested_locale = Some(locale.to_string());
                document.content_locale = Some(version.target_locale.clone());
                document.translation_status = Some(TranslationStatus::Succeeded);
                document.is_fallback = false;
                continue;
            }

            let status = statuses_by_document.get(document_id);
            document.requested_locale = Some(locale.to_string());
            document.content_locale = status
                .and_then(|value| value.source_locale.clone())
                .or_else(|| Some("original".to_string()));
            document.translation_status = Some(match status.map(|value| value.status.as_str()) {
                Some("queued") => TranslationStatus::Queued,
                Some("running") => TranslationStatus::Running,
                Some("failed") => TranslationStatus::Failed,
                Some("skipped") => TranslationStatus::Skipped,
                Some("quota_exceeded") => TranslationStatus::QuotaExceeded,
                Some("succeeded") => TranslationStatus::Succeeded,
                _ => TranslationStatus::Unavailable,
            });
            document.is_fallback = true;
        }

        Ok(result)
    }

    pub async fn document_chunk_ids(&self, document_id: i64) -> Result<Vec<Uuid>> {
        Ok(
            sqlx::query_file_scalar!("src/sql/db/documents/list_chunk_ids.sql", document_id)
                .fetch_all(&self.pool)
                .await?,
        )
    }

    pub async fn delete_document_by_id(&self, document_id: i64) -> Result<()> {
        sqlx::query_file!("src/sql/db/documents/delete_by_id.sql", document_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
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
        let mut metadata_keys = Vec::new();
        let mut metadata_values = Vec::new();
        for definition in self
            .list_metadata_indexes(payload.group_id, &payload.source_key)
            .await?
            .into_iter()
            .filter(|definition| definition.status == "ready")
        {
            metadata_keys.push((definition.index_id, document_id));
            let values = crate::services::document_store::metadata::extract_values(
                &definition,
                &payload.metadata_json,
            )?;
            metadata_values.extend(crate::db::metadata_indexes::metadata_value_rows(
                definition.index_id,
                document_id,
                &values,
            ));
        }
        self.replace_metadata_values_bulk(&metadata_keys, &metadata_values)
            .await?;
        Ok(UpsertedDocument {
            document_id,
            changed,
        })
    }

    pub async fn update_library_document_business_fields(
        &self,
        document_id: i64,
        payload: &ChunkPayload,
    ) -> Result<()> {
        let definitions = self
            .list_metadata_indexes(payload.group_id, &payload.source_key)
            .await?
            .into_iter()
            .filter(|definition| definition.status == "ready")
            .map(|definition| {
                let values = crate::services::document_store::metadata::extract_values(
                    &definition,
                    &payload.metadata_json,
                )?;
                Ok((definition.index_id, values))
            })
            .collect::<Result<Vec<_>>>()?;

        let mut tx = self.pool.begin().await?;
        super::document_versions::ensure_library_version_in_transaction(
            &mut tx,
            document_id,
            payload,
        )
        .await?;
        sqlx::query_file!(
            "src/sql/db/documents/update_library_business_fields.sql",
            document_id,
            payload.external_id,
            payload.source_uri,
            payload.published_at,
            payload.updated_at_source,
            payload.metadata_json,
            payload.record_hash
        )
        .execute(&mut *tx)
        .await?;
        let metadata_keys = definitions
            .iter()
            .map(|(index_id, _)| (*index_id, document_id))
            .collect::<Vec<_>>();
        let metadata_values = definitions
            .into_iter()
            .flat_map(|(index_id, values)| {
                crate::db::metadata_indexes::metadata_value_rows(index_id, document_id, &values)
            })
            .collect::<Vec<_>>();
        crate::db::metadata_indexes::replace_metadata_values_in_transaction(
            &mut tx,
            &metadata_keys,
            &metadata_values,
        )
        .await?;
        tx.commit().await?;
        Ok(())
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

        insert_document_chunks_in_transaction(&mut tx, document_id, record_hash, chunks).await?;

        tx.commit().await?;
        Ok(chunks.iter().map(|chunk| chunk.id).collect())
    }

    pub async fn delete_document_chunks(&self, document_id: i64) -> Result<()> {
        sqlx::query_file!(
            "src/sql/db/documents/delete_document_chunks.sql",
            document_id
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn insert_document_chunks(
        &self,
        document_id: i64,
        record_hash: &str,
        chunks: &[DocumentChunk],
    ) -> Result<()> {
        if chunks.is_empty() {
            return Ok(());
        }

        let mut tx = self.pool.begin().await?;
        insert_document_chunks_in_transaction(&mut tx, document_id, record_hash, chunks).await?;
        tx.commit().await?;
        Ok(())
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
        requested_locale: Option<&str>,
        scope: &AccessScope,
    ) -> Result<HashMap<Uuid, SearchHit>> {
        let rows = sqlx::query_file_as!(
            SearchHitRow,
            "src/sql/db/documents/fetch_search_hits_by_chunk_ids.sql",
            chunk_ids,
            &scope.private_group_ids,
            scope.group_path.as_deref(),
            requested_locale
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
                        requested_locale: requested_locale.map(ToOwned::to_owned),
                        content_locale: Some(row.content_locale.clone()),
                        translation_status: translation_status(row.translation_status.as_deref()),
                        is_fallback: requested_locale.is_some() && row.content_locale == "original",
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
        let metadata_filters = serde_json::to_value(&request.metadata_filters)?;
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
            keyword_limit,
            request.locale.as_deref(),
            metadata_filters
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| search_hit_from_keyword_row(row, request.locale.as_deref()))
            .collect())
    }

    pub async fn count_chunk_payloads_for_reindex(&self) -> Result<usize> {
        let count =
            sqlx::query_file_scalar!("src/sql/db/documents/count_chunk_payloads_for_reindex.sql")
                .fetch_one(&self.pool)
                .await?;
        usize::try_from(count).context("reindex chunk count is out of range")
    }

    pub async fn list_chunk_payloads_for_reindex_page(
        &self,
        last_document_id: Option<i64>,
        last_chunk_index: Option<i32>,
        limit: i64,
    ) -> Result<Vec<ChunkPayload>> {
        let rows = sqlx::query_file_as!(
            ReindexChunkRow,
            "src/sql/db/documents/list_chunk_payloads_for_reindex_page.sql",
            last_document_id,
            last_chunk_index,
            limit
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(reindex_payload_from_row).collect())
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

        Ok(Some(document_response_from_parts(
            document,
            chunks
                .into_iter()
                .filter(|chunk| is_meaningful_text(&chunk.chunk_text))
                .map(|chunk| DocumentChunkResponse {
                    chunk_id: chunk.id,
                    chunk_index: chunk.chunk_index,
                    text: chunk.chunk_text,
                })
                .collect(),
            None,
            None,
            false,
        )))
    }
}

fn document_response_from_parts(
    document: DocumentRow,
    chunks: Vec<DocumentChunkResponse>,
    requested_locale: Option<String>,
    content_locale: Option<String>,
    is_fallback: bool,
) -> DocumentResponse {
    DocumentResponse {
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
        chunks,
        requested_locale,
        content_locale,
        translation_status: None,
        is_fallback,
    }
}

fn reindex_payload_from_row(row: ReindexChunkRow) -> ChunkPayload {
    ChunkPayload {
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
        content_locale: "original".to_string(),
        source_locale: None,
        translation_provider: None,
    }
}

async fn insert_document_chunks_in_transaction(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    document_id: i64,
    record_hash: &str,
    chunks: &[DocumentChunk],
) -> Result<()> {
    if chunks.is_empty() {
        return Ok(());
    }
    let chunk_ids = chunks.iter().map(|chunk| chunk.id).collect::<Vec<_>>();
    let chunk_indexes = chunks
        .iter()
        .map(|chunk| chunk.chunk_index)
        .collect::<Vec<_>>();
    let chunk_texts = chunks
        .iter()
        .map(|chunk| chunk.text.clone())
        .collect::<Vec<_>>();
    sqlx::query_file!(
        "src/sql/db/documents/insert_document_chunks_bulk.sql",
        document_id,
        record_hash,
        &chunk_ids,
        &chunk_indexes,
        &chunk_texts
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}
