use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

/// Legacy direct-path row selected by the migration tool: the file still
/// references its old UUID-path object directly (no storage object yet).
#[derive(Debug, Clone, FromRow)]
pub struct LegacyDirectPathFileRow {
    pub id: Uuid,
    pub group_id: i64,
    pub filename: String,
    pub size_bytes: i64,
    pub sha256: String,
    pub storage_rel_path: String,
    pub created_at: DateTime<Utc>,
}

/// Current storage linkage of a library_files row, used to classify why a
/// conditional legacy reference update did not land.
#[derive(Debug, FromRow)]
pub struct LegacyFileStorageStateRow {
    pub storage_object_id: Option<Uuid>,
    pub storage_rel_path: String,
}

/// Legacy direct-path row eligible for the startup missing-source cleanup:
/// still pointing at the legacy key, in a terminal ingest state, and old
/// enough that the active storage key has had a chance to settle.
#[derive(Debug, Clone, FromRow)]
pub struct MissingLegacySourceFileRow {
    pub id: Uuid,
    pub group_id: i64,
    pub filename: String,
    pub size_bytes: i64,
    pub sha256: String,
    pub storage_rel_path: String,
    pub ingest_status: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, FromRow)]
struct FileTranslationDirectiveRow {
    translation_override: bool,
    translation_source_locale: Option<String>,
    translation_target_locales: Vec<String>,
}

use super::mappers::file_from_row;
use super::{
    FileRow, LibraryFileRecord, LibraryIngestStatus, LibraryStore, NewLibraryFile, TaskWriteDenied,
    TaskWriteFence, UpdateLibraryFileContent,
};

impl LibraryStore {
    pub async fn set_file_translation_directive(
        &self,
        file_id: Uuid,
        directive: Option<&crate::contracts::TranslationDirective>,
    ) -> Result<()> {
        sqlx::query_file!(
            "src/sql/library_store/files/set_translation_directive.sql",
            file_id,
            directive.is_some(),
            directive.and_then(|value| value.source_locale.as_deref()),
            directive
                .map(|value| value.target_locales.as_slice())
                .unwrap_or_default()
        )
        .execute(self.db.pool())
        .await?;
        Ok(())
    }

    pub async fn file_translation_directive(
        &self,
        file_id: Uuid,
    ) -> Result<Option<crate::contracts::TranslationDirective>> {
        let row = sqlx::query_file_as!(
            FileTranslationDirectiveRow,
            "src/sql/library_store/files/get_translation_directive.sql",
            file_id
        )
        .fetch_optional(self.db.pool())
        .await?;
        Ok(row.and_then(|row| {
            row.translation_override
                .then_some(crate::contracts::TranslationDirective {
                    source_locale: row.translation_source_locale,
                    target_locales: row.translation_target_locales,
                })
        }))
    }

    pub async fn update_business_metadata(
        &self,
        group_id: i64,
        file_id: Uuid,
        metadata: &crate::contracts::CanonicalUploadMetadata,
    ) -> Result<Option<LibraryFileRecord>> {
        let metadata_json = crate::contracts::metadata_object_to_value(&metadata.metadata_json);
        let row = sqlx::query_file_as!(
            FileRow,
            "src/sql/library_store/files/update_business_metadata.sql",
            group_id,
            file_id,
            metadata.external_id,
            metadata.source_uri,
            metadata.published_at,
            metadata_json
        )
        .fetch_optional(self.db.pool())
        .await?;
        row.map(file_from_row).transpose()
    }
    pub async fn list_files(&self) -> Result<Vec<LibraryFileRecord>> {
        let rows = sqlx::query_file_as!(FileRow, "src/sql/library_store/files/list_files.sql")
            .fetch_all(self.db.pool())
            .await?;

        rows.into_iter().map(file_from_row).collect()
    }

    /// Bounded page of legacy direct-path rows ordered by the
    /// `(created_at, id)` cursor. Pass `None` for both cursor values to start
    /// from the beginning.
    pub async fn list_legacy_direct_path_files(
        &self,
        after_created_at: Option<DateTime<Utc>>,
        after_id: Option<Uuid>,
        limit: i64,
    ) -> Result<Vec<LegacyDirectPathFileRow>> {
        Ok(sqlx::query_file_as!(
            LegacyDirectPathFileRow,
            "src/sql/library_store/files/list_legacy_direct_path_files.sql",
            after_created_at,
            after_id,
            limit
        )
        .fetch_all(self.db.pool())
        .await?)
    }

    /// Bounded page of legacy direct-path rows whose source object is a
    /// candidate for startup missing-source cleanup. Filters out rows that
    /// were concurrently linked (`storage_object_id IS NOT NULL`), ingested
    /// again (non-terminal `ingest_status`), or are too recent to safely
    /// verify (younger than `grace_hours`).
    pub async fn list_missing_legacy_source_files(
        &self,
        grace_hours: i64,
        after_created_at: Option<DateTime<Utc>>,
        after_id: Option<Uuid>,
        limit: i64,
    ) -> Result<Vec<MissingLegacySourceFileRow>> {
        Ok(sqlx::query_file_as!(
            MissingLegacySourceFileRow,
            "src/sql/library_store/files/list_missing_legacy_source_files.sql",
            grace_hours,
            after_created_at,
            after_id,
            limit
        )
        .fetch_all(self.db.pool())
        .await?)
    }

    /// Lightweight existence check for any legacy direct-path rows. Used by
    /// startup to gate the old-key cleanup phase: legacy old-key deletion
    /// only runs when this returns `false` so a partially-completed
    /// migration cannot strand old objects that the missing-source cleanup
    /// is still about to act on.
    pub async fn has_legacy_direct_path_files(&self) -> Result<bool> {
        let rows = self
            .list_legacy_direct_path_files(None, None, 1_i64)
            .await?;
        Ok(!rows.is_empty())
    }

    /// Conditionally link a legacy direct-path row to a content-addressed
    /// storage object. Returns `false` when the row no longer matches the old
    /// key or was linked concurrently; the caller must then treat its own
    /// object as unreferenced.
    pub async fn link_legacy_file_storage_object_on_connection(
        &self,
        connection: &mut sqlx::PgConnection,
        file_id: Uuid,
        expected_old_key: &str,
        object_id: Uuid,
        object_key: &str,
    ) -> Result<bool> {
        Ok(sqlx::query_file!(
            "src/sql/library_store/files/link_legacy_file_storage_object.sql",
            file_id,
            object_id,
            object_key,
            expected_old_key
        )
        .fetch_optional(connection)
        .await?
        .is_some())
    }

    pub async fn get_legacy_file_storage_state(
        &self,
        file_id: Uuid,
    ) -> Result<Option<LegacyFileStorageStateRow>> {
        Ok(sqlx::query_file_as!(
            LegacyFileStorageStateRow,
            "src/sql/library_store/files/get_legacy_file_storage_state.sql",
            file_id
        )
        .fetch_optional(self.db.pool())
        .await?)
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

    pub async fn list_filenames_in_project_folder(
        &self,
        project_id: i64,
        folder_id: Option<Uuid>,
        exclude_file_id: Option<Uuid>,
    ) -> Result<Vec<String>> {
        Ok(sqlx::query_file_scalar!(
            "src/sql/library_store/files/list_filenames_in_project_folder.sql",
            project_id,
            folder_id,
            exclude_file_id
        )
        .fetch_all(self.db.pool())
        .await?)
    }

    pub async fn get_file(&self, file_id: Uuid) -> Result<Option<LibraryFileRecord>> {
        let row =
            sqlx::query_file_as!(FileRow, "src/sql/library_store/files/get_file.sql", file_id)
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

    pub async fn get_file_by_sha_in_project(
        &self,
        project_id: i64,
        sha256: &str,
    ) -> Result<Option<LibraryFileRecord>> {
        let row = sqlx::query_file_as!(
            FileRow,
            "src/sql/library_store/files/get_file_by_sha_in_project.sql",
            project_id,
            sha256
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
            file.storage_rel_path,
            file.storage_object_id,
            file.delete_source_after_processing
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
            project_id,
            file.storage_object_id,
            file.delete_source_after_processing
        )
        .fetch_one(self.db.pool())
        .await?;

        file_from_row(row)
    }

    /// The create statement inside the fenced write of a running task item.
    ///
    /// The library row is written on the connection of the guarded transaction,
    /// so the file and the grant that authorized it commit together. A refused
    /// grant returns `None` and nothing is written.
    pub async fn create_file_in_project_fenced(
        &self,
        fence: TaskWriteFence,
        project_id: i64,
        file: &NewLibraryFile,
    ) -> Result<Option<LibraryFileRecord>> {
        let mut write = match self.begin_task_write(fence).await {
            Ok(write) => write,
            Err(error) if error.downcast_ref::<TaskWriteDenied>().is_some() => return Ok(None),
            Err(error) => return Err(error),
        };
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
            project_id,
            file.storage_object_id,
            file.delete_source_after_processing
        )
        .fetch_optional(write.connection())
        .await?;
        let Some(row) = row else {
            write.rollback().await;
            return Ok(None);
        };
        let record = file_from_row(row)?;
        if !write.commit().await? {
            return Ok(None);
        }
        Ok(Some(record))
    }

    /// The content update inside the fenced write of a running task item.
    pub async fn update_file_content_in_project_fenced(
        &self,
        fence: TaskWriteFence,
        project_id: i64,
        file_id: Uuid,
        update: &UpdateLibraryFileContent,
    ) -> Result<Option<LibraryFileRecord>> {
        let mut write = match self.begin_task_write(fence).await {
            Ok(write) => write,
            Err(error) if error.downcast_ref::<TaskWriteDenied>().is_some() => return Ok(None),
            Err(error) => return Err(error),
        };
        let row = sqlx::query_file_as!(
            FileRow,
            "src/sql/library_store/files/update_file_content_in_project.sql",
            project_id,
            file_id,
            update.folder_id,
            update.external_id,
            update.filename,
            update.media_type,
            update.size_bytes,
            update.sha256,
            update.storage_rel_path,
            update.storage_object_id
        )
        .fetch_optional(write.connection())
        .await?;
        let Some(row) = row else {
            write.rollback().await;
            return Ok(None);
        };
        let record = file_from_row(row)?;
        if !write.commit().await? {
            return Ok(None);
        }
        Ok(Some(record))
    }

    pub async fn update_file_content_in_project(
        &self,
        project_id: i64,
        file_id: Uuid,
        update: &UpdateLibraryFileContent,
    ) -> Result<Option<LibraryFileRecord>> {
        let row = sqlx::query_file_as!(
            FileRow,
            "src/sql/library_store/files/update_file_content_in_project.sql",
            project_id,
            file_id,
            update.folder_id,
            update.external_id,
            update.filename,
            update.media_type,
            update.size_bytes,
            update.sha256,
            update.storage_rel_path,
            update.storage_object_id
        )
        .fetch_optional(self.db.pool())
        .await?;

        row.map(file_from_row).transpose()
    }

    pub async fn restore_file_snapshot_in_project(
        &self,
        file: &crate::domain::LibraryFileRecord,
        storage_object_id: Option<Uuid>,
        source_released_at: Option<DateTime<Utc>>,
    ) -> Result<Option<LibraryFileRecord>> {
        let row = sqlx::query_file_as!(
            FileRow,
            "src/sql/library_store/files/restore_file_snapshot_in_project.sql",
            file.group_id,
            file.id,
            file.folder_id,
            file.external_id,
            file.filename,
            file.media_type,
            file.size_bytes,
            file.sha256,
            file.storage_rel_path,
            storage_object_id,
            file.ingest_status.as_str(),
            file.error_message,
            file.ingested_at,
            source_released_at
        )
        .fetch_optional(self.db.pool())
        .await?;

        row.map(file_from_row).transpose()
    }

    /// The compensation of one failed task upload, in one guarded write.
    ///
    /// The previous snapshot of the file, its business metadata, its
    /// translation directive and the search generation counter are restored on
    /// the connection of the guarded transaction of the *original* captured
    /// grant, and the grant is checked again before the commit. `false` means
    /// the grant is gone: nothing is restored, and the caller must not fall back
    /// to the pooled helpers, because the row may already belong to a newer
    /// owner.
    pub async fn restore_file_snapshot_for_task_fenced(
        &self,
        fence: TaskWriteFence,
        file: &crate::domain::LibraryFileRecord,
        storage_object_id: Option<Uuid>,
        source_released_at: Option<DateTime<Utc>>,
        metadata: &crate::contracts::CanonicalUploadMetadata,
        translation: Option<&crate::contracts::TranslationDirective>,
    ) -> Result<bool> {
        let mut write = match self.begin_task_write(fence).await {
            Ok(write) => write,
            Err(error) if error.downcast_ref::<TaskWriteDenied>().is_some() => return Ok(false),
            Err(error) => return Err(error),
        };
        let metadata_json = crate::contracts::metadata_object_to_value(&metadata.metadata_json);
        let restored = sqlx::query_file_as!(
            FileRow,
            "src/sql/library_store/files/restore_file_snapshot_in_project.sql",
            file.group_id,
            file.id,
            file.folder_id,
            file.external_id,
            file.filename,
            file.media_type,
            file.size_bytes,
            file.sha256,
            file.storage_rel_path,
            storage_object_id,
            file.ingest_status.as_str(),
            file.error_message,
            file.ingested_at,
            source_released_at
        )
        .fetch_optional(write.connection())
        .await?;
        if restored.is_none() {
            write.rollback().await;
            return Ok(false);
        }
        let updated = sqlx::query_file_as!(
            FileRow,
            "src/sql/library_store/files/update_business_metadata.sql",
            file.group_id,
            file.id,
            metadata.external_id,
            metadata.source_uri,
            metadata.published_at,
            metadata_json
        )
        .fetch_optional(write.connection())
        .await?;
        if updated.is_none() {
            write.rollback().await;
            return Ok(false);
        }
        sqlx::query_file!(
            "src/sql/library_store/files/set_translation_directive.sql",
            file.id,
            translation.is_some(),
            translation.and_then(|value| value.source_locale.as_deref()),
            translation
                .map(|value| value.target_locales.as_slice())
                .unwrap_or_default()
        )
        .execute(write.connection())
        .await?;
        // The counter is advanced on the guarded connection with the existing
        // checked statement, so the restored snapshot and the cache
        // invalidation it implies commit together.
        sqlx::query_file_scalar!("src/sql/db/search_cache/bump_search_generation.sql")
            .fetch_one(write.connection())
            .await?;
        write.commit().await
    }

    /// Removes the file record of one failed task upload under its grant.
    ///
    /// `false` means the grant is gone and nothing was removed.
    pub async fn rollback_file_record_for_task_fenced(
        &self,
        fence: TaskWriteFence,
        project_id: i64,
        file_id: Uuid,
    ) -> Result<bool> {
        self.delete_file_in_project_fenced(fence, project_id, file_id)
            .await
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

    /// The status projection of one file, written inside the guarded write of
    /// a running task item.
    ///
    /// A worker that lost its segment projects nothing: the statement runs on
    /// the connection of the guarded transaction, and the grant is rechecked
    /// before the commit.
    pub async fn update_file_status_fenced(
        &self,
        fence: TaskWriteFence,
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
        let mut write = match self.begin_task_write(fence).await {
            Ok(write) => write,
            Err(error) if error.downcast_ref::<TaskWriteDenied>().is_some() => return Ok(None),
            Err(error) => return Err(error),
        };
        let row = sqlx::query_file_as!(
            FileRow,
            "src/sql/library_store/files/update_file_status.sql",
            file_id,
            status.as_str(),
            error_message,
            ingested_at
        )
        .fetch_optional(write.connection())
        .await?;
        let Some(row) = row else {
            write.rollback().await;
            return Ok(None);
        };
        let record = file_from_row(row)?;
        if !write.commit().await? {
            return Ok(None);
        }
        Ok(Some(record))
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

    /// Delete one file row under the captured grant of a running task item.
    ///
    /// The documents of the file and the file row are removed in one guarded
    /// transaction, so a worker that lost its segment cannot delete anything and
    /// a crash between the two deletes cannot leave a file without documents.
    pub async fn delete_file_in_project_fenced(
        &self,
        fence: TaskWriteFence,
        project_id: i64,
        file_id: Uuid,
    ) -> Result<bool> {
        let mut write = match self.begin_task_write(fence).await {
            Ok(write) => write,
            Err(error) if error.downcast_ref::<TaskWriteDenied>().is_some() => return Ok(false),
            Err(error) => return Err(error),
        };
        self.delete_documents_for_files_in(write.connection(), &[file_id])
            .await?;
        let deleted = sqlx::query_file!(
            "src/sql/library_store/files/delete_file_record_in_project.sql",
            project_id,
            file_id
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
