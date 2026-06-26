use anyhow::Result;
use uuid::Uuid;

use super::{ChunkPayloadRow, FileDocumentRow, LibraryStore};
use crate::domain::ChunkPayload;
use crate::domain::LibraryFileDocumentRecord;

#[derive(Debug)]
struct StoragePathRow {
    id: Uuid,
    storage_rel_path: String,
}

impl LibraryStore {
    pub async fn replace_file_documents(
        &self,
        file_id: Uuid,
        documents: &[LibraryFileDocumentRecord],
    ) -> Result<()> {
        let mut tx = self.db.pool().begin().await?;
        sqlx::query_file!("src/sql/library_store/documents/delete_file_documents.sql", file_id)
            .execute(&mut *tx)
            .await?;

        for document in documents {
            sqlx::query_file!(
                "src/sql/library_store/documents/insert_file_document.sql",
                file_id,
                document.document_id,
                document.group_id,
                document.project_id,
                document.visibility.as_str(),
                document.section_key,
                document.section_label,
                document.sort_order
            )
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
        let rows = sqlx::query_file_as!(
            FileDocumentRow,
            "src/sql/library_store/documents/list_file_documents.sql",
            file_id
        )
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

        Ok(
            sqlx::query_file_scalar!(
                "src/sql/library_store/documents/list_document_ids_for_files.sql",
                file_ids
            )
            .fetch_all(self.db.pool())
            .await?,
        )
    }

    pub async fn list_chunk_ids_for_files(&self, file_ids: &[Uuid]) -> Result<Vec<Uuid>> {
        if file_ids.is_empty() {
            return Ok(Vec::new());
        }

        Ok(
            sqlx::query_file_scalar!(
                "src/sql/library_store/documents/list_chunk_ids_for_files.sql",
                file_ids
            )
            .fetch_all(self.db.pool())
            .await?,
        )
    }

    pub async fn list_chunk_payloads_for_files(
        &self,
        file_ids: &[Uuid],
    ) -> Result<Vec<ChunkPayload>> {
        if file_ids.is_empty() {
            return Ok(Vec::new());
        }

        let rows = sqlx::query_file_as!(
            ChunkPayloadRow,
            "src/sql/library_store/documents/list_chunk_payloads_for_files.sql",
            file_ids
        )
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

        sqlx::query_file!(
            "src/sql/library_store/documents/delete_documents_for_files.sql",
            file_ids
        )
        .execute(self.db.pool())
        .await?;
        Ok(())
    }

    pub async fn list_storage_paths_for_files(
        &self,
        file_ids: &[Uuid],
    ) -> Result<Vec<(Uuid, String)>> {
        if file_ids.is_empty() {
            return Ok(Vec::new());
        }

        let rows = sqlx::query_file_as!(
            StoragePathRow,
            "src/sql/library_store/documents/list_storage_paths_for_files.sql",
            file_ids
        )
        .fetch_all(self.db.pool())
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| (row.id, row.storage_rel_path))
            .collect())
    }

    pub async fn update_document_metadata(
        &self,
        document_id: i64,
        metadata_json: &serde_json::Value,
    ) -> Result<()> {
        sqlx::query_file!(
            "src/sql/library_store/documents/update_document_metadata.sql",
            document_id,
            metadata_json
        )
        .execute(self.db.pool())
        .await?;
        Ok(())
    }
}
