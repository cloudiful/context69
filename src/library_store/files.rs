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

#[derive(Debug, FromRow)]
struct FileTranslationDirectiveRow {
    translation_override: bool,
    translation_source_locale: Option<String>,
    translation_target_locales: Vec<String>,
}

#[derive(Debug, FromRow)]
struct FileExtractionDirectiveRow {
    extraction_template_key: Option<String>,
    extraction_parameters: serde_json::Value,
}

use super::mappers::file_from_row;
use super::{
    FileRow, LibraryFileRecord, LibraryIngestStatus, LibraryStore, NewLibraryFile,
    UpdateLibraryFileContent,
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

    pub async fn set_file_extraction_directive(
        &self,
        file_id: Uuid,
        directive: Option<&crate::contracts::ExtractionDirective>,
    ) -> Result<()> {
        sqlx::query_file!(
            "src/sql/library_store/files/set_extraction_directive.sql",
            file_id,
            directive.map(|value| value.template_key.as_str()),
            directive
                .map(|value| value.parameters.clone())
                .unwrap_or_else(|| serde_json::json!({}))
        )
        .execute(self.db.pool())
        .await?;
        Ok(())
    }

    pub async fn file_extraction_directive(
        &self,
        file_id: Uuid,
    ) -> Result<Option<crate::contracts::ExtractionDirective>> {
        let row = sqlx::query_file_as!(
            FileExtractionDirectiveRow,
            "src/sql/library_store/files/get_extraction_directive.sql",
            file_id
        )
        .fetch_optional(self.db.pool())
        .await?;
        Ok(row.and_then(|row| {
            let template_key = row.extraction_template_key?;
            Some(crate::contracts::ExtractionDirective {
                template_key,
                parameters: row.extraction_parameters,
            })
        }))
    }

    pub async fn update_business_metadata(
        &self,
        group_id: i64,
        file_id: Uuid,
        metadata: &crate::contracts::LibraryFileUploadMetadata,
    ) -> Result<Option<LibraryFileRecord>> {
        let row = sqlx::query_file_as!(
            FileRow,
            "src/sql/library_store/files/update_business_metadata.sql",
            group_id,
            file_id,
            metadata.external_id,
            metadata.source_uri,
            metadata.published_at,
            metadata.metadata_json
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
            file.storage_object_id
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
            file.storage_object_id
        )
        .fetch_one(self.db.pool())
        .await?;

        file_from_row(row)
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
            file.ingested_at
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
