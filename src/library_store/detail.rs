use anyhow::Result;
use uuid::Uuid;

use super::{
    FileDetailRow, LibraryDocumentSectionPreview, LibraryFileDetailResponse, LibraryStore,
    SectionPreviewRow, infer_preview_content_format,
};
use crate::contracts::Visibility;
use crate::normalize::is_meaningful_text;

impl LibraryStore {
    pub async fn get_file_detail(
        &self,
        file_id: Uuid,
        folder_path: String,
    ) -> Result<Option<LibraryFileDetailResponse>> {
        let file = sqlx::query_file_as!(
            FileDetailRow,
            "src/sql/library_store/files/get_file_detail.sql",
            file_id
        )
        .fetch_optional(self.db.pool())
        .await?;

        let Some(file) = file else {
            return Ok(None);
        };

        let sections = self.list_section_previews_for_file(file_id).await?;
        let jobs = self
            .list_jobs_for_file(file_id)
            .await?
            .into_iter()
            .map(super::job_to_response)
            .collect();

        Ok(Some(LibraryFileDetailResponse {
            file_id: file.file_id,
            group_key: file.group_key,
            group_path: file.group_path,
            visibility: file.visibility.parse().unwrap_or(Visibility::Private),
            folder_id: file.folder_id,
            folder_path,
            filename: file.filename.clone(),
            media_type: file.media_type.clone(),
            size_bytes: file.size_bytes,
            sha256: file.sha256,
            ingest_status: file.ingest_status.parse()?,
            error_message: file.error_message,
            created_at: file.created_at,
            updated_at: file.updated_at,
            ingested_at: file.ingested_at,
            sections,
            jobs,
        }))
    }

    pub async fn get_file_detail_in_project(
        &self,
        project_id: i64,
        file_id: Uuid,
        folder_path: String,
    ) -> Result<Option<LibraryFileDetailResponse>> {
        let file = sqlx::query_file_as!(
            FileDetailRow,
            "src/sql/library_store/files/get_file_detail_in_project.sql",
            project_id,
            file_id
        )
        .fetch_optional(self.db.pool())
        .await?;

        let Some(file) = file else {
            return Ok(None);
        };

        let sections = self.list_section_previews_for_file(file_id).await?;
        let jobs = self
            .list_jobs_for_file(file_id)
            .await?
            .into_iter()
            .map(super::job_to_response)
            .collect();

        Ok(Some(LibraryFileDetailResponse {
            file_id: file.file_id,
            group_key: file.group_key,
            group_path: file.group_path,
            visibility: file.visibility.parse().unwrap_or(Visibility::Private),
            folder_id: file.folder_id,
            folder_path,
            filename: file.filename.clone(),
            media_type: file.media_type.clone(),
            size_bytes: file.size_bytes,
            sha256: file.sha256,
            ingest_status: file.ingest_status.parse()?,
            error_message: file.error_message,
            created_at: file.created_at,
            updated_at: file.updated_at,
            ingested_at: file.ingested_at,
            sections,
            jobs,
        }))
    }

    pub async fn list_section_previews_for_file(
        &self,
        file_id: Uuid,
    ) -> Result<Vec<LibraryDocumentSectionPreview>> {
        let rows = sqlx::query_as::<_, SectionPreviewRow>(
            r#"
            SELECT
                lfd.document_id,
                lfd.section_key,
                lfd.section_label,
                lfd.sort_order,
                d.title,
                lf.media_type,
                (
                    SELECT dc.chunk_text
                    FROM context69.document_chunks dc
                    WHERE dc.document_id = d.id
                      AND dc.chunk_text IS NOT NULL
                      AND length(trim(dc.chunk_text)) > 0
                    ORDER BY dc.chunk_index
                    LIMIT 1
                ) AS chunk_text
            FROM context69.library_file_documents lfd
            INNER JOIN context69.documents d ON d.id = lfd.document_id
            INNER JOIN context69.library_files lf ON lf.id = lfd.file_id
            WHERE lfd.file_id = $1
            ORDER BY lfd.sort_order ASC, lfd.section_key ASC
            "#,
        )
        .bind(file_id)
        .fetch_all(self.db.pool())
        .await?;

        Ok(rows
            .into_iter()
            .map(|row| LibraryDocumentSectionPreview {
                content_format: infer_preview_content_format(&row.title, &row.media_type),
                document_id: row.document_id,
                section_key: row.section_key,
                section_label: row.section_label,
                sort_order: row.sort_order,
                title: row.title.clone(),
                preview_text: row
                    .chunk_text
                    .filter(|value| is_meaningful_text(value))
                    .unwrap_or_default(),
            })
            .collect())
    }
}
