use anyhow::Result;
use chrono::Utc;
use uuid::Uuid;

use super::mappers::file_from_row;
use super::{
    FileRow, LibraryFileRecord, LibraryIngestStatus, LibraryStore, NewLibraryFile,
    UpdateLibraryTextFile,
};

impl LibraryStore {
    pub async fn list_files(&self) -> Result<Vec<LibraryFileRecord>> {
        let rows = sqlx::query_file_as!(FileRow, "src/sql/library_store/files/list_files.sql")
            .fetch_all(self.db.pool())
            .await?;

        rows.into_iter().map(file_from_row).collect()
    }

    pub async fn list_files_by_ids(&self, file_ids: &[Uuid]) -> Result<Vec<LibraryFileRecord>> {
        if file_ids.is_empty() {
            return Ok(Vec::new());
        }

        let rows = sqlx::query_file_as!(
            FileRow,
            "src/sql/library_store/files/list_files_by_ids.sql",
            file_ids
        )
            .fetch_all(self.db.pool())
            .await?;

        rows.into_iter().map(file_from_row).collect()
    }

    pub async fn list_files_in_project(&self, project_id: i64) -> Result<Vec<LibraryFileRecord>> {
        let rows = sqlx::query_file_as!(
            FileRow,
            "src/sql/library_store/files/list_files_in_project.sql",
            project_id
        )
            .fetch_all(self.db.pool())
            .await?;

        rows.into_iter().map(file_from_row).collect()
    }

    pub async fn get_file(&self, file_id: Uuid) -> Result<Option<LibraryFileRecord>> {
        let row = sqlx::query_file_as!(FileRow, "src/sql/library_store/files/get_file.sql", file_id)
            .fetch_optional(self.db.pool())
            .await?;

        row.map(file_from_row).transpose()
    }

    pub async fn get_file_in_project(
        &self,
        project_id: i64,
        file_id: Uuid,
    ) -> Result<Option<LibraryFileRecord>> {
        let row = sqlx::query_file_as!(
            FileRow,
            "src/sql/library_store/files/get_file_in_project.sql",
            project_id,
            file_id
        )
            .fetch_optional(self.db.pool())
            .await?;

        row.map(file_from_row).transpose()
    }

    pub async fn get_file_by_external_id_in_project(
        &self,
        project_id: i64,
        external_id: &str,
    ) -> Result<Option<LibraryFileRecord>> {
        let row = sqlx::query_file_as!(
            FileRow,
            "src/sql/library_store/files/get_file_by_external_id_in_project.sql",
            project_id,
            external_id
        )
            .fetch_optional(self.db.pool())
            .await?;

        row.map(file_from_row).transpose()
    }

    pub async fn create_file(&self, file: &NewLibraryFile) -> Result<LibraryFileRecord> {
        let row = sqlx::query_file_as!(
            FileRow,
            "src/sql/library_store/files/create_file.sql",
            file.id,
            file.folder_id,
            file.external_id,
            file.filename,
            file.media_type,
            file.size_bytes,
            file.sha256,
            file.storage_rel_path
        )
            .fetch_one(self.db.pool())
            .await?;

        file_from_row(row)
    }

    pub async fn create_file_in_project(
        &self,
        project_id: i64,
        file: &NewLibraryFile,
    ) -> Result<LibraryFileRecord> {
        let row = sqlx::query_file_as!(
            FileRow,
            "src/sql/library_store/files/create_file_in_project.sql",
            file.id,
            file.folder_id,
            file.external_id,
            file.filename,
            file.media_type,
            file.size_bytes,
            file.sha256,
            file.storage_rel_path,
            project_id
        )
            .fetch_one(self.db.pool())
            .await?;

        file_from_row(row)
    }

    pub async fn update_text_file_in_project(
        &self,
        project_id: i64,
        file_id: Uuid,
        update: &UpdateLibraryTextFile,
    ) -> Result<Option<LibraryFileRecord>> {
        let row = sqlx::query_file_as!(
            FileRow,
            "src/sql/library_store/files/update_text_file_in_project.sql",
            project_id,
            file_id,
            update.folder_id,
            update.external_id,
            update.filename,
            update.media_type,
            update.size_bytes,
            update.sha256,
            update.storage_rel_path
        )
            .fetch_optional(self.db.pool())
            .await?;

        row.map(file_from_row).transpose()
    }

    pub async fn move_file(
        &self,
        file_id: Uuid,
        target_folder_id: Option<Uuid>,
    ) -> Result<Option<LibraryFileRecord>> {
        let row = sqlx::query_file_as!(
            FileRow,
            "src/sql/library_store/files/move_file.sql",
            file_id,
            target_folder_id
        )
            .fetch_optional(self.db.pool())
            .await?;

        row.map(file_from_row).transpose()
    }

    pub async fn move_file_in_project(
        &self,
        project_id: i64,
        file_id: Uuid,
        target_folder_id: Option<Uuid>,
    ) -> Result<Option<LibraryFileRecord>> {
        let row = sqlx::query_file_as!(
            FileRow,
            "src/sql/library_store/files/move_file_in_project.sql",
            project_id,
            file_id,
            target_folder_id
        )
            .fetch_optional(self.db.pool())
            .await?;

        row.map(file_from_row).transpose()
    }

    pub async fn update_file_status(
        &self,
        file_id: Uuid,
        status: LibraryIngestStatus,
        error_message: Option<&str>,
        mark_ingested_now: bool,
    ) -> Result<Option<LibraryFileRecord>> {
        let ingested_at = if mark_ingested_now {
            Some(Utc::now())
        } else {
            None
        };
        let row = sqlx::query_file_as!(
            FileRow,
            "src/sql/library_store/files/update_file_status.sql",
            file_id,
            status.as_str(),
            error_message,
            ingested_at
        )
            .fetch_optional(self.db.pool())
            .await?;

        row.map(file_from_row).transpose()
    }

    pub async fn delete_file_record(&self, file_id: Uuid) -> Result<bool> {
        let result = sqlx::query_file!(
            "src/sql/library_store/files/delete_file_record.sql",
            file_id
        )
            .execute(self.db.pool())
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn delete_file_record_in_project(
        &self,
        project_id: i64,
        file_id: Uuid,
    ) -> Result<bool> {
        let result = sqlx::query_file!(
            "src/sql/library_store/files/delete_file_record_in_project.sql",
            project_id,
            file_id
        )
        .execute(self.db.pool())
        .await?;
        Ok(result.rows_affected() > 0)
    }
}
