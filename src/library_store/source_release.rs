//! Store access for the library source-file lifecycle (issue 332 phase 2).
//!
//! `delete_source_after_processing` and `source_released_at` are deliberately
//! kept off [`super::FileRow`] so the hot file-listing queries stay unchanged;
//! the release path reads them through the small, focused queries here.

use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgConnection};
use uuid::Uuid;

use super::LibraryStore;

/// Source-file lifecycle state of one `library_files` row.
#[derive(Debug, Clone, FromRow)]
pub struct FileSourceLifecycleRow {
    pub id: Uuid,
    pub group_id: i64,
    pub filename: String,
    pub external_id: Option<String>,
    pub ingest_status: String,
    pub storage_rel_path: String,
    pub storage_object_id: Option<Uuid>,
    pub sha256: String,
    pub size_bytes: i64,
    pub delete_source_after_processing: bool,
    pub source_released_at: Option<DateTime<Utc>>,
}

/// One opted-in succeeded file whose source has not been released yet.
#[derive(Debug, Clone, FromRow)]
pub struct PendingAutoReleaseFile {
    pub id: Uuid,
    pub group_id: i64,
}

impl LibraryStore {
    pub async fn get_file_source_lifecycle(
        &self,
        file_id: Uuid,
    ) -> Result<Option<FileSourceLifecycleRow>> {
        Ok(sqlx::query_file_as!(
            FileSourceLifecycleRow,
            "src/sql/library_store/source_release/get_file_source_lifecycle.sql",
            file_id
        )
        .fetch_optional(self.db.pool())
        .await?)
    }

    pub async fn lock_file_source_lifecycle(
        &self,
        connection: &mut PgConnection,
        file_id: Uuid,
    ) -> Result<Option<FileSourceLifecycleRow>> {
        Ok(sqlx::query_file_as!(
            FileSourceLifecycleRow,
            "src/sql/library_store/source_release/lock_file_source_lifecycle.sql",
            file_id
        )
        .fetch_optional(connection)
        .await?)
    }

    /// Idempotently mark the source deliberately released. Returns `false`
    /// when another transaction already released it.
    pub async fn mark_file_source_released(
        &self,
        connection: &mut PgConnection,
        file_id: Uuid,
    ) -> Result<bool> {
        Ok(sqlx::query_file!(
            "src/sql/library_store/source_release/mark_file_source_released.sql",
            file_id
        )
        .fetch_optional(connection)
        .await?
        .is_some())
    }

    pub async fn count_active_file_items(
        &self,
        connection: &mut PgConnection,
        file_id: Uuid,
    ) -> Result<i64> {
        Ok(sqlx::query_file_scalar!(
            "src/sql/library_store/source_release/count_active_file_items.sql",
            file_id
        )
        .fetch_one(connection)
        .await?)
    }

    pub async fn count_files_referencing_storage_path(
        &self,
        connection: &mut PgConnection,
        storage_rel_path: &str,
        excluding_file_id: Uuid,
    ) -> Result<i64> {
        Ok(sqlx::query_file_scalar!(
            "src/sql/library_store/source_release/count_files_referencing_storage_path.sql",
            storage_rel_path,
            excluding_file_id
        )
        .fetch_one(connection)
        .await?)
    }

    pub async fn list_pending_auto_release_files(
        &self,
        limit: i64,
    ) -> Result<Vec<PendingAutoReleaseFile>> {
        Ok(sqlx::query_file_as!(
            PendingAutoReleaseFile,
            "src/sql/library_store/source_release/list_pending_auto_release_files.sql",
            limit
        )
        .fetch_all(self.db.pool())
        .await?)
    }
}
