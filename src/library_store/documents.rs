use anyhow::Result;
use uuid::Uuid;

use super::{ChunkPayloadRow, FileDocumentRow, LibraryStore};
use crate::domain::ChunkPayload;
use crate::domain::LibraryFileDocumentRecord;

impl LibraryStore {
    pub async fn replace_file_documents(
        &self,
        file_id: Uuid,
        documents: &[LibraryFileDocumentRecord],
    ) -> Result<()> {
        let mut tx = self.db.pool().begin().await?;
        sqlx::query("DELETE FROM context69.library_file_documents WHERE file_id = $1")
            .bind(file_id)
            .execute(&mut *tx)
            .await?;

        for document in documents {
            sqlx::query(
                r#"
                INSERT INTO context69.library_file_documents (
                    file_id,
                    document_id,
                    group_id,
                    project_id,
                    visibility,
                    section_key,
                    section_label,
                    sort_order
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                "#,
            )
            .bind(file_id)
            .bind(document.document_id)
            .bind(document.group_id)
            .bind(document.project_id)
            .bind(document.visibility.as_str())
            .bind(&document.section_key)
            .bind(&document.section_label)
            .bind(document.sort_order)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    pub async fn list_file_documents(
        &self,
        file_id: Uuid,
    ) -> Result<Vec<LibraryFileDocumentRecord>> {
        let rows = sqlx::query_as::<_, FileDocumentRow>(
            r#"
            SELECT file_id, document_id, group_id, project_id, visibility, section_key, section_label, sort_order
            FROM context69.library_file_documents
            WHERE file_id = $1
            ORDER BY sort_order ASC, section_key ASC
            "#,
        )
        .bind(file_id)
        .fetch_all(self.db.pool())
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| LibraryFileDocumentRecord {
                file_id: row.file_id,
                document_id: row.document_id,
                group_id: row.group_id,
                project_id: row.project_id,
                visibility: row
                    .visibility
                    .parse()
                    .unwrap_or(crate::contracts::Visibility::Private),
                section_key: row.section_key,
                section_label: row.section_label,
                sort_order: row.sort_order,
            })
            .collect())
    }

    pub async fn list_document_ids_for_files(&self, file_ids: &[Uuid]) -> Result<Vec<i64>> {
        if file_ids.is_empty() {
            return Ok(Vec::new());
        }

        Ok(sqlx::query_scalar::<_, i64>(
            r#"
            SELECT DISTINCT document_id
            FROM context69.library_file_documents
            WHERE file_id = ANY($1)
            ORDER BY document_id
            "#,
        )
        .bind(file_ids)
        .fetch_all(self.db.pool())
        .await?)
    }

    pub async fn list_chunk_ids_for_files(&self, file_ids: &[Uuid]) -> Result<Vec<Uuid>> {
        if file_ids.is_empty() {
            return Ok(Vec::new());
        }

        Ok(sqlx::query_scalar::<_, Uuid>(
            r#"
            SELECT c.id
            FROM context69.document_chunks c
            INNER JOIN context69.library_file_documents lfd ON lfd.document_id = c.document_id
            WHERE lfd.file_id = ANY($1)
            ORDER BY c.document_id, c.chunk_index
            "#,
        )
        .bind(file_ids)
        .fetch_all(self.db.pool())
        .await?)
    }

    pub async fn list_chunk_payloads_for_files(
        &self,
        file_ids: &[Uuid],
    ) -> Result<Vec<ChunkPayload>> {
        if file_ids.is_empty() {
            return Ok(Vec::new());
        }

        let rows = sqlx::query_as::<_, ChunkPayloadRow>(
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
            INNER JOIN context69.library_file_documents lfd ON lfd.document_id = d.id
            WHERE lfd.file_id = ANY($1)
            ORDER BY d.id, c.chunk_index
            "#,
        )
        .bind(file_ids)
        .fetch_all(self.db.pool())
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
                visibility: row
                    .visibility
                    .parse()
                    .unwrap_or(crate::contracts::Visibility::Private),
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

    pub async fn delete_documents_for_files(&self, file_ids: &[Uuid]) -> Result<()> {
        if file_ids.is_empty() {
            return Ok(());
        }

        sqlx::query(
            r#"
            DELETE FROM context69.documents
            WHERE id IN (
                SELECT DISTINCT document_id
                FROM context69.library_file_documents
                WHERE file_id = ANY($1)
            )
            "#,
        )
        .bind(file_ids)
        .execute(self.db.pool())
        .await?;
        Ok(())
    }

    pub async fn list_storage_paths_for_files(&self, file_ids: &[Uuid]) -> Result<Vec<(Uuid, String)>> {
        if file_ids.is_empty() {
            return Ok(Vec::new());
        }

        let rows = sqlx::query_as::<_, (Uuid, String)>(
            r#"
            SELECT id, storage_rel_path
            FROM context69.library_files
            WHERE id = ANY($1)
            ORDER BY filename, id
            "#,
        )
        .bind(file_ids)
        .fetch_all(self.db.pool())
        .await?;

        Ok(rows)
    }

    pub async fn update_document_metadata(
        &self,
        document_id: i64,
        metadata_json: &serde_json::Value,
    ) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE context69.documents
            SET metadata_json = $2, updated_at = now()
            WHERE id = $1
            "#,
        )
        .bind(document_id)
        .bind(metadata_json)
        .execute(self.db.pool())
        .await?;
        Ok(())
    }
}
