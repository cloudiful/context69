use anyhow::Result;
use sqlx::FromRow;
use uuid::Uuid;

use super::mappers::folder_from_row;
use super::{FolderRow, LibraryFolderRecord, LibraryStore, TaskWriteDenied, TaskWriteFence};

/// The library folder row a delete locks.
///
/// The group is the scope of the delete: a project-scoped delete never locks
/// (and never deletes) a folder of another project.
#[derive(Debug, Clone, PartialEq, FromRow)]
pub struct DeleteFolderRow {
    pub id: Uuid,
    pub group_id: i64,
    pub parent_id: Option<Uuid>,
}

impl LibraryStore {
    pub async fn list_folders(&self) -> Result<Vec<LibraryFolderRecord>> {
        let mut connection = self.db.pool().acquire().await?;
        self.list_folders_on(&mut connection).await
    }

    /// The folder rows on a caller transaction.
    ///
    /// The metadata publisher reads the folder path on the same connection as
    /// the file and document writes, so no pooled read observes another
    /// transaction between them.
    pub async fn list_folders_on(
        &self,
        connection: &mut sqlx::PgConnection,
    ) -> Result<Vec<LibraryFolderRecord>> {
        let rows =
            sqlx::query_file_as!(FolderRow, "src/sql/library_store/folders/list_folders.sql")
                .fetch_all(connection)
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

    /// The folder move on a caller transaction.
    pub async fn move_folder_on(
        &self,
        connection: &mut sqlx::PgConnection,
        folder_id: Uuid,
        target_folder_id: Option<Uuid>,
    ) -> Result<Option<LibraryFolderRecord>> {
        let row = sqlx::query_file_as!(
            FolderRow,
            "src/sql/library_store/folders/move_folder.sql",
            folder_id,
            target_folder_id
        )
        .fetch_optional(connection)
        .await?;

        row.map(folder_from_row).transpose()
    }

    /// The project-scoped folder move on a caller transaction.
    pub async fn move_folder_in_project_on(
        &self,
        connection: &mut sqlx::PgConnection,
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
        .fetch_optional(connection)
        .await?;

        row.map(folder_from_row).transpose()
    }

    /// Every file under a folder subtree, selected on the caller connection.
    ///
    /// The recursive query runs in the same transaction as the folder move,
    /// so a later refusal cannot leave the folder moved with only part of its
    /// descendant metadata refreshed.
    pub async fn descendant_file_ids_on(
        &self,
        connection: &mut sqlx::PgConnection,
        folder_id: Uuid,
    ) -> Result<Vec<Uuid>> {
        Ok(sqlx::query_file_scalar!(
            "src/sql/library_store/folders/descendant_file_ids.sql",
            folder_id
        )
        .fetch_all(connection)
        .await?)
    }

    /// Every file under a project-scoped folder subtree on the caller
    /// connection.
    pub async fn descendant_file_ids_in_project_on(
        &self,
        connection: &mut sqlx::PgConnection,
        group_id: i64,
        folder_id: Uuid,
    ) -> Result<Vec<Uuid>> {
        Ok(sqlx::query_file_scalar!(
            "src/sql/library_store/folders/descendant_file_ids_in_project.sql",
            group_id,
            folder_id
        )
        .fetch_all(connection)
        .await?)
    }

    /// Delete one folder row under the captured grant of a running task item.
    ///
    /// The folder row is removed in the guarded transaction, so a worker that
    /// lost its segment cannot delete the folder even when it already removed
    /// the file rows it enumerated.
    pub async fn delete_folder_record_fenced(
        &self,
        fence: TaskWriteFence,
        folder_id: Uuid,
    ) -> Result<bool> {
        let mut write = match self.begin_task_write(fence).await {
            Ok(write) => write,
            Err(error) if error.downcast_ref::<TaskWriteDenied>().is_some() => return Ok(false),
            Err(error) => return Err(error),
        };
        let deleted = sqlx::query_file!(
            "src/sql/library_store/folders/delete_folder_record.sql",
            folder_id
        )
        .execute(write.connection())
        .await?
        .rows_affected()
            > 0;
        if !deleted {
            write.rollback().await;
            return Ok(false);
        }
        write.commit().await
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

    /// Read and lock one folder row for a delete.
    ///
    /// The folder delete locks the registry epoch first and this row second,
    /// so a folder delete never holds a folder lock while it waits for the
    /// epoch. `project_id` scopes the lock to one group.
    pub async fn lock_folder_for_delete_on(
        &self,
        connection: &mut sqlx::PgConnection,
        folder_id: Uuid,
        project_id: Option<i64>,
    ) -> Result<Option<DeleteFolderRow>> {
        let row = match project_id {
            Some(project_id) => {
                sqlx::query_file_as!(
                    DeleteFolderRow,
                    "src/sql/library_store/folders/lock_folder_for_delete_in_project.sql",
                    folder_id,
                    project_id
                )
                .fetch_optional(connection)
                .await?
            }
            None => {
                sqlx::query_file_as!(
                    DeleteFolderRow,
                    "src/sql/library_store/folders/lock_folder_for_delete.sql",
                    folder_id
                )
                .fetch_optional(connection)
                .await?
            }
        };
        Ok(row)
    }

    /// Delete one folder row on a borrowed connection.
    ///
    /// The folder row owns its files, so the delete removes the files of the
    /// subtree in the same transaction and the delete backstop trigger of every
    /// file records its entity tombstone there.
    pub async fn delete_folder_record_on(
        &self,
        connection: &mut sqlx::PgConnection,
        folder_id: Uuid,
    ) -> Result<bool> {
        let result = sqlx::query_file!(
            "src/sql/library_store/folders/delete_folder_record.sql",
            folder_id
        )
        .execute(connection)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn folder_processing_counts(
        &self,
        project_id: Option<i64>,
    ) -> Result<std::collections::HashMap<Option<Uuid>, usize>> {
        #[derive(sqlx::FromRow)]
        struct ProcessingCountRow {
            folder_id: Option<Uuid>,
            count: i64,
        }
        let rows = sqlx::query_file_as!(
            ProcessingCountRow,
            "src/sql/library_store/folders/count_processing_by_folder.sql",
            project_id,
        )
        .fetch_all(self.db.pool())
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| (row.folder_id, usize::try_from(row.count).unwrap_or(0)))
            .collect())
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

    /// Every ancestor of one folder, including the folder itself, on the
    /// caller's transaction.
    ///
    /// The recursive step uses distinct-set semantics, so a pre-existing
    /// malformed cycle still terminates. A move refuses when the candidate
    /// parent is already an ancestor of the folder being moved.
    pub async fn folder_ancestor_ids_on(
        &self,
        connection: &mut sqlx::PgConnection,
        folder_id: Uuid,
    ) -> Result<Vec<Uuid>> {
        Ok(sqlx::query_file_scalar!(
            "src/sql/library_store/folders/folder_ancestor_ids.sql",
            folder_id
        )
        .fetch_all(connection)
        .await?)
    }
}
