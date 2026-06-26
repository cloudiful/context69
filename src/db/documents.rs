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
        let existing = sqlx::query_as::<_, ExistingDocumentRow>(
            r#"
            SELECT id, record_hash
            FROM context69.documents
            WHERE project_id = $1 AND source_key = $2 AND external_id = $3
            "#,
        )
        .bind(payload.project_id)
        .bind(&payload.source_key)
        .bind(&payload.external_id)
        .fetch_optional(&mut *tx)
        .await?;

        let (document_id, changed) = match existing {
            Some(existing) if existing.record_hash == payload.record_hash => {
                sqlx::query(
                    r#"
                    UPDATE context69.documents
                    SET title = $3,
                        summary = $4,
                        source_uri = $5,
                        published_at = $6,
                        updated_at_source = COALESCE($7, updated_at_source),
                        metadata_json = $8,
                        last_synced_at = now(),
                        updated_at = now()
                    WHERE project_id = $1 AND source_key = $2 AND external_id = $9
                    "#,
                )
                .bind(payload.project_id)
                .bind(&payload.source_key)
                .bind(&payload.title)
                .bind(&payload.summary)
                .bind(&payload.source_uri)
                .bind(payload.published_at)
                .bind(payload.updated_at_source)
                .bind(&payload.metadata_json)
                .bind(&payload.external_id)
                .execute(&mut *tx)
                .await?;
                (existing.id, false)
            }
            Some(existing) => {
                sqlx::query(
                    r#"
                    UPDATE context69.documents
                    SET title = $3,
                        summary = $4,
                        source_uri = $5,
                        published_at = $6,
                        updated_at_source = COALESCE($7, updated_at_source),
                        metadata_json = $8,
                        record_hash = $9,
                        last_synced_at = now(),
                        updated_at = now()
                    WHERE project_id = $1 AND source_key = $2 AND external_id = $10
                    "#,
                )
                .bind(payload.project_id)
                .bind(&payload.source_key)
                .bind(&payload.title)
                .bind(&payload.summary)
                .bind(&payload.source_uri)
                .bind(payload.published_at)
                .bind(payload.updated_at_source)
                .bind(&payload.metadata_json)
                .bind(&payload.record_hash)
                .bind(&payload.external_id)
                .execute(&mut *tx)
                .await?;
                (existing.id, true)
            }
            None => {
                let id = sqlx::query_scalar::<_, i64>(
                    r#"
                    INSERT INTO context69.documents (
                        group_id,
                        project_id,
                        visibility,
                        source_key,
                        external_id,
                        title,
                        summary,
                        source_uri,
                        published_at,
                        updated_at_source,
                        metadata_json,
                        record_hash
                    )
                    VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
                    RETURNING id
                    "#,
                )
                .bind(payload.group_id)
                .bind(payload.project_id)
                .bind(payload.visibility.as_str())
                .bind(&payload.source_key)
                .bind(&payload.external_id)
                .bind(&payload.title)
                .bind(&payload.summary)
                .bind(&payload.source_uri)
                .bind(payload.published_at)
                .bind(payload.updated_at_source)
                .bind(&payload.metadata_json)
                .bind(&payload.record_hash)
                .fetch_one(&mut *tx)
                .await?;
                (id, true)
            }
        };

        if changed {
            sqlx::query(
                r#"
                INSERT INTO context69.document_versions (
                    document_id,
                    record_hash,
                    title,
                    summary,
                    body_text,
                    source_uri,
                    published_at,
                    metadata_json
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                ON CONFLICT (document_id, record_hash) DO NOTHING
                "#,
            )
            .bind(document_id)
            .bind(&payload.record_hash)
            .bind(&payload.title)
            .bind(&payload.summary)
            .bind(&payload.chunk_text)
            .bind(&payload.source_uri)
            .bind(payload.published_at)
            .bind(&payload.metadata_json)
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
        sqlx::query("DELETE FROM context69.document_chunks WHERE document_id = $1")
            .bind(document_id)
            .execute(&mut *tx)
            .await?;

        for chunk in chunks {
            sqlx::query(
                r#"
                INSERT INTO context69.document_chunks (
                    id,
                    document_id,
                    chunk_index,
                    chunk_text,
                    record_hash
                )
                VALUES ($1, $2, $3, $4, $5)
                "#,
            )
            .bind(chunk.id)
            .bind(document_id)
            .bind(chunk.chunk_index)
            .bind(&chunk.text)
            .bind(record_hash)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(chunks.iter().map(|chunk| chunk.id).collect())
    }

    pub async fn list_chunk_ids_for_document(&self, document_id: i64) -> Result<Vec<Uuid>> {
        Ok(sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM context69.document_chunks WHERE document_id = $1 ORDER BY chunk_index",
        )
        .bind(document_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn fetch_search_hits_by_chunk_ids(
        &self,
        chunk_ids: &[Uuid],
        scope: &AccessScope,
    ) -> Result<HashMap<Uuid, SearchHit>> {
        let rows = sqlx::query_as::<_, SearchHitRow>(
            r#"
            SELECT
                c.id AS chunk_id,
                d.id AS document_id,
                g.group_key,
                p.project_key,
                d.visibility,
                d.source_key,
                d.external_id,
                d.title,
                d.summary,
                d.source_uri,
                d.published_at,
                c.chunk_index,
                c.chunk_text,
                d.metadata_json
            FROM context69.document_chunks c
            INNER JOIN context69.documents d ON d.id = c.document_id
            INNER JOIN context69.groups g ON g.id = d.group_id
            INNER JOIN context69.projects p ON p.id = d.project_id
            WHERE c.id = ANY($1)
              AND (d.visibility = 'public' OR d.project_id = ANY($2))
              AND ($3::text IS NULL OR g.group_key = $3)
              AND ($4::text IS NULL OR p.project_key = $4)
            "#,
        )
        .bind(chunk_ids)
        .bind(&scope.private_project_ids)
        .bind(scope.group_key.as_deref())
        .bind(scope.project_key.as_deref())
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
                        project_key: row.project_key,
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

        let rows = sqlx::query_as::<_, KeywordSearchHitRow>(
            r#"
            WITH query_terms AS (
                SELECT unnest($3::text[]) AS term
            ),
            scored AS (
                SELECT
                    c.id AS chunk_id,
                    d.id AS document_id,
                    g.group_key,
                    p.project_key,
                    d.visibility,
                    d.source_key,
                    d.external_id,
                    d.title,
                    d.summary,
                    d.source_uri,
                    d.published_at,
                    c.chunk_index,
                    c.chunk_text,
                    d.metadata_json,
                    lower(d.title) AS title_lc,
                    lower(c.chunk_text) AS chunk_lc
                FROM context69.document_chunks c
                INNER JOIN context69.documents d ON d.id = c.document_id
                INNER JOIN context69.groups g ON g.id = d.group_id
                INNER JOIN context69.projects p ON p.id = d.project_id
                WHERE ($4::text IS NULL OR d.source_key = $4)
                  AND ($5::text IS NULL OR g.group_key = $5)
                  AND ($6::text IS NULL OR p.project_key = $6)
                  AND ($7::date IS NULL OR d.published_at >= $7)
                  AND ($8::date IS NULL OR d.published_at <= $8)
                  AND (d.visibility = 'public' OR d.project_id = ANY($9))
                  AND (
                    lower(d.title) LIKE $2
                    OR lower(c.chunk_text) LIKE $2
                    OR (
                        cardinality($3::text[]) > 0
                        AND NOT EXISTS (
                            SELECT 1
                            FROM query_terms qt
                            WHERE (lower(d.title) || ' ' || lower(c.chunk_text)) NOT LIKE ('%' || qt.term || '%')
                        )
                    )
                  )
            )
            SELECT
                chunk_id,
                document_id,
                group_key,
                project_key,
                visibility,
                source_key,
                external_id,
                title,
                summary,
                source_uri,
                published_at,
                chunk_index,
                chunk_text,
                metadata_json,
                (
                    CASE WHEN title_lc = $1 THEN 1.20 ELSE 0 END
                    + CASE WHEN title_lc LIKE $2 THEN 1.00 ELSE 0 END
                    + CASE WHEN chunk_lc LIKE $2 THEN 0.82 ELSE 0 END
                    + CASE WHEN cardinality($3::text[]) > 0 THEN 0.35 ELSE 0 END
                )::real AS keyword_score,
                CASE
                    WHEN title_lc = $1 THEN 'title_exact'
                    WHEN title_lc LIKE $2 THEN 'title_phrase'
                    WHEN chunk_lc LIKE $2 THEN 'chunk_phrase'
                    ELSE 'all_terms'
                END AS match_reason
            FROM scored
            ORDER BY keyword_score DESC, published_at DESC NULLS LAST, document_id DESC, chunk_index ASC
            LIMIT $10
            "#,
        )
        .bind(&normalized_query)
        .bind(&phrase_pattern)
        .bind(&terms)
        .bind(&request.source_key)
        .bind(request.group_key.as_deref())
        .bind(request.project_key.as_deref())
        .bind(request.published_after)
        .bind(request.published_before)
        .bind(&scope.private_project_ids)
        .bind(i64::try_from(limit).context("keyword search limit is too large")?)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(search_hit_from_keyword_row).collect())
    }

    pub async fn list_chunk_payloads_for_reindex(&self) -> Result<Vec<ChunkPayload>> {
        let rows = sqlx::query_as::<_, ReindexChunkRow>(
            r#"
            SELECT
                c.id AS chunk_id,
                d.id AS document_id,
                d.group_id,
                g.group_key,
                d.project_id,
                p.project_key,
                d.visibility,
                d.source_key,
                d.external_id,
                d.title,
                d.summary,
                d.source_uri,
                d.published_at,
                d.updated_at_source,
                d.record_hash,
                c.chunk_index,
                c.chunk_text,
                d.metadata_json
            FROM context69.document_chunks c
            INNER JOIN context69.documents d ON d.id = c.document_id
            INNER JOIN context69.groups g ON g.id = d.group_id
            INNER JOIN context69.projects p ON p.id = d.project_id
            ORDER BY d.id, c.chunk_index
            "#,
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
                project_id: row.project_id,
                project_key: row.project_key,
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
        let document = sqlx::query_as::<_, DocumentRow>(
            r#"
            SELECT
                id,
                g.group_key,
                p.project_key,
                d.visibility,
                source_key,
                external_id,
                title,
                summary,
                source_uri,
                published_at,
                updated_at_source,
                record_hash,
                metadata_json
            FROM context69.documents d
            INNER JOIN context69.groups g ON g.id = d.group_id
            INNER JOIN context69.projects p ON p.id = d.project_id
            WHERE d.id = $1
              AND (d.visibility = 'public' OR d.project_id = ANY($2))
              AND ($3::text IS NULL OR g.group_key = $3)
              AND ($4::text IS NULL OR p.project_key = $4)
            "#,
        )
        .bind(document_id)
        .bind(&scope.private_project_ids)
        .bind(scope.group_key.as_deref())
        .bind(scope.project_key.as_deref())
        .fetch_optional(&self.pool)
        .await?;

        let Some(document) = document else {
            return Ok(None);
        };

        let chunks = sqlx::query_as::<_, ChunkRow>(
            r#"
            SELECT id, chunk_index, chunk_text
            FROM context69.document_chunks
            WHERE document_id = $1
            ORDER BY chunk_index
            "#,
        )
        .bind(document_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(Some(DocumentResponse {
            document_id: document.id,
            group_key: document.group_key,
            project_key: document.project_key,
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
