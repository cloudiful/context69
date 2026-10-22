use anyhow::Result;
use uuid::Uuid;

use super::{
    ChunkPayloadRow, FileDocumentRow, LibraryIngestStatus, LibraryStore, TaskWriteDenied,
    TaskWriteFence,
};
use crate::chunk_payload::{RowPayload, from_row};
use crate::domain::ChunkPayload;
use crate::domain::LibraryFileDocumentRecord;

#[derive(Debug)]
pub struct StoragePathRow {
    pub id: Uuid,
    pub storage_rel_path: String,
    pub storage_object_id: Option<Uuid>,
    /// Set when the source was deliberately released; the physical bytes are
    /// then owned by the storage-object/ref-count path, never by this row.
    pub source_released_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl LibraryStore {
    pub async fn list_chunk_ids_for_library_file(&self, file_id: Uuid) -> Result<Vec<Uuid>> {
        Ok(sqlx::query_file_scalar!(
            "src/sql/library_store/documents/list_chunk_ids_for_library_file.sql",
            file_id.to_string()
        )
        .fetch_all(self.db.pool())
        .await?)
    }

    /// The documents of one library file, deleted inside the guarded write of
    /// a running task item.
    ///
    /// The delete runs on the connection of the guarded transaction, so the
    /// cleanup and the grant that authorized it commit together.
    pub async fn delete_documents_for_library_file_fenced(
        &self,
        fence: TaskWriteFence,
        file_id: Uuid,
    ) -> Result<bool> {
        let mut write = match self.begin_task_write(fence).await {
            Ok(write) => write,
            Err(error) if error.downcast_ref::<TaskWriteDenied>().is_some() => return Ok(false),
            Err(error) => return Err(error),
        };
        sqlx::query_file!(
            "src/sql/library_store/task_fence/delete_documents_for_library_file.sql",
            file_id.to_string()
        )
        .execute(write.connection())
        .await?;
        write.commit().await
    }

    pub async fn delete_documents_for_library_file(&self, file_id: Uuid) -> Result<()> {
        sqlx::query_file!(
            "src/sql/library_store/documents/delete_documents_for_library_file.sql",
            file_id.to_string()
        )
        .execute(self.db.pool())
        .await?;
        Ok(())
    }

    pub async fn replace_file_documents(
        &self,
        file_id: Uuid,
        documents: &[LibraryFileDocumentRecord],
    ) -> Result<()> {
        let mut tx = self.db.pool().begin().await?;
        replace_file_documents_on(&mut tx, file_id, documents).await?;
        tx.commit().await?;
        Ok(())
    }

    /// The links of one file and its status, in one guarded write.
    ///
    /// The links are replaced and the status of the file is projected on the
    /// connection of the guarded transaction, so a file never carries the links
    /// of one segment and the status of another.
    pub async fn replace_file_documents_and_status_fenced(
        &self,
        fence: TaskWriteFence,
        file_id: Uuid,
        documents: &[LibraryFileDocumentRecord],
        status: LibraryIngestStatus,
        error_message: Option<&str>,
        mark_ingested_now: bool,
    ) -> Result<bool> {
        let mut write = match self.begin_task_write(fence).await {
            Ok(write) => write,
            Err(error) if error.downcast_ref::<TaskWriteDenied>().is_some() => return Ok(false),
            Err(error) => return Err(error),
        };
        replace_file_documents_on(write.connection(), file_id, documents).await?;
        let ingested_at = if mark_ingested_now {
            Some(chrono::Utc::now())
        } else {
            None
        };
        sqlx::query_file_as!(
            super::FileRow,
            "src/sql/library_store/files/update_file_status.sql",
            file_id,
            status.as_str(),
            error_message,
            ingested_at
        )
        .fetch_optional(write.connection())
        .await?;
        write.commit().await
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
                visibility: row
                    .visibility
                    .parse()
                    .unwrap_or(crate::contracts::Visibility::Private),
                section_key: row.section_key,
                section_label: row.section_label,
                section_external_id: row.section_external_id,
                section_source_uri: row.section_source_uri,
                section_published_at: row.section_published_at,
                section_metadata_json: row.section_metadata_json,
                sort_order: row.sort_order,
            })
            .collect())
    }

    pub async fn list_chunk_ids_for_files(&self, file_ids: &[Uuid]) -> Result<Vec<Uuid>> {
        if file_ids.is_empty() {
            return Ok(Vec::new());
        }

        Ok(sqlx::query_file_scalar!(
            "src/sql/library_store/documents/list_chunk_ids_for_files.sql",
            file_ids
        )
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

        let rows = sqlx::query_file_as!(
            ChunkPayloadRow,
            "src/sql/library_store/documents/list_chunk_payloads_for_files.sql",
            file_ids
        )
        .fetch_all(self.db.pool())
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| {
                from_row(RowPayload {
                    chunk_id: row.chunk_id,
                    document_id: row.document_id,
                    group_id: row.group_id,
                    group_key: row.group_key,
                    group_path: row.group_path,
                    visibility: row.visibility,
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
            })
            .collect())
    }

    pub async fn delete_documents_for_files(&self, file_ids: &[Uuid]) -> Result<()> {
        if file_ids.is_empty() {
            return Ok(());
        }
        let mut connection = self.db.pool().acquire().await?;
        self.delete_documents_for_files_in(&mut connection, file_ids)
            .await
    }

    /// The document delete on a borrowed connection.
    ///
    /// A task-owned destructive write runs on the connection of its fenced
    /// transaction, so the documents and the grant that authorized their
    /// removal commit together.
    pub async fn delete_documents_for_files_in(
        &self,
        connection: &mut sqlx::PgConnection,
        file_ids: &[Uuid],
    ) -> Result<()> {
        if file_ids.is_empty() {
            return Ok(());
        }
        sqlx::query_file!(
            "src/sql/library_store/documents/delete_documents_for_files.sql",
            file_ids
        )
        .execute(connection)
        .await?;
        Ok(())
    }

    pub async fn list_storage_paths_for_files(
        &self,
        file_ids: &[Uuid],
    ) -> Result<Vec<StoragePathRow>> {
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

        Ok(rows)
    }
}

/// The links of one file, replaced on a borrowed connection.
async fn replace_file_documents_on(
    connection: &mut sqlx::PgConnection,
    file_id: Uuid,
    documents: &[LibraryFileDocumentRecord],
) -> Result<()> {
    sqlx::query_file!(
        "src/sql/library_store/documents/delete_file_documents.sql",
        file_id
    )
    .execute(&mut *connection)
    .await?;

    for document in documents {
        sqlx::query_file!(
            "src/sql/library_store/documents/insert_file_document.sql",
            file_id,
            document.document_id,
            document.group_id,
            document.visibility.as_str(),
            document.section_key,
            document.section_label,
            document.section_external_id,
            document.section_source_uri,
            document.section_published_at,
            document.section_metadata_json,
            document.sort_order
        )
        .execute(&mut *connection)
        .await?;
    }
    Ok(())
}
