use anyhow::Result;
use uuid::Uuid;

use super::mappers::folder_from_row;
use super::{FolderRow, LibraryFolderRecord, LibraryStore};

impl LibraryStore {
    pub async fn list_folders(&self) -> Result<Vec<LibraryFolderRecord>> {
        let rows =
            sqlx::query_file_as!(FolderRow, "src/sql/library_store/folders/list_folders.sql")
                .fetch_all(self.db.pool())
                .await?;

        rows.into_iter().map(folder_from_row).collect()
    }

    pub async fn get_folder(&self, folder_id: Uuid) -> Result<Option<LibraryFolderRecord>> {
        let row = sqlx::query_file_as!(
            FolderRow,
            "src/sql/library_store/folders/get_folder.sql",
            folder_id
        )
        .fetch_optional(self.db.pool())
        .await?;

        row.map(folder_from_row).transpose()
    }

    pub async fn list_folders_in_project(
        &self,
        project_id: i64,
    ) -> Result<Vec<LibraryFolderRecord>> {
        let rows = sqlx::query_file_as!(
            FolderRow,
            "src/sql/library_store/folders/list_folders_in_project.sql",
            project_id
        )
        .fetch_all(self.db.pool())
        .await?;

        rows.into_iter().map(folder_from_row).collect()
    }

    pub async fn get_folder_in_project(
        &self,
        project_id: i64,
        folder_id: Uuid,
    ) -> Result<Option<LibraryFolderRecord>> {
        let row = sqlx::query_file_as!(
            FolderRow,
            "src/sql/library_store/folders/get_folder_in_project.sql",
            project_id,
            folder_id
        )
        .fetch_optional(self.db.pool())
        .await?;

        row.map(folder_from_row).transpose()
    }

    pub async fn create_folder(
        &self,
        folder_id: Uuid,
        parent_folder_id: Option<Uuid>,
        name: &str,
    ) -> Result<LibraryFolderRecord> {
        let row = sqlx::query_file_as!(
            FolderRow,
            "src/sql/library_store/folders/create_folder.sql",
            folder_id,
            parent_folder_id,
            name
        )
        .fetch_one(self.db.pool())
        .await?;

        folder_from_row(row)
    }

    pub async fn create_folder_in_project(
        &self,
        project_id: i64,
        folder_id: Uuid,
        parent_folder_id: Option<Uuid>,
        name: &str,
    ) -> Result<LibraryFolderRecord> {
        let row = sqlx::query_file_as!(
            FolderRow,
            "src/sql/library_store/folders/create_folder_in_project.sql",
            folder_id,
            parent_folder_id,
            name,
            project_id
        )
        .fetch_one(self.db.pool())
        .await?;

        folder_from_row(row)
    }

    pub async fn move_folder(
        &self,
        folder_id: Uuid,
        target_folder_id: Option<Uuid>,
    ) -> Result<Option<LibraryFolderRecord>> {
        let row = sqlx::query_file_as!(
            FolderRow,
            "src/sql/library_store/folders/move_folder.sql",
            folder_id,
            target_folder_id
        )
        .fetch_optional(self.db.pool())
        .await?;

        row.map(folder_from_row).transpose()
    }

    pub async fn move_folder_in_project(
        &self,
        project_id: i64,
        folder_id: Uuid,
        target_folder_id: Option<Uuid>,
    ) -> Result<Option<LibraryFolderRecord>> {
        let row = sqlx::query_file_as!(
            FolderRow,
            "src/sql/library_store/folders/move_folder_in_project.sql",
            project_id,
            folder_id,
            target_folder_id
        )
        .fetch_optional(self.db.pool())
        .await?;

        row.map(folder_from_row).transpose()
    }

    pub async fn delete_folder_record(&self, folder_id: Uuid) -> Result<bool> {
        let result = sqlx::query_file!(
            "src/sql/library_store/folders/delete_folder_record.sql",
            folder_id
        )
            .execute(self.db.pool())
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn descendant_folder_ids(&self, folder_id: Uuid) -> Result<Vec<Uuid>> {
        let rows = sqlx::query_file_scalar!(
            "src/sql/library_store/folders/descendant_folder_ids.sql",
            folder_id
        )
        .fetch_all(self.db.pool())
        .await?;
        Ok(rows)
    }

    pub async fn descendant_folder_ids_in_project(
        &self,
        project_id: i64,
        folder_id: Uuid,
    ) -> Result<Vec<Uuid>> {
        let rows = sqlx::query_file_scalar!(
            "src/sql/library_store/folders/descendant_folder_ids_in_project.sql",
            project_id,
            folder_id
        )
        .fetch_all(self.db.pool())
        .await?;
        Ok(rows)
    }
}
