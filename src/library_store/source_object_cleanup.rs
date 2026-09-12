//! Durable physical-deletion outbox for released source objects (issue 332
//! phase 2, attempt 2).
//!
//! The release transaction records an intent here and leaves the
//! storage-object row untouched. The cleanup worker deletes the bytes first
//! and the row second, so every failure is retryable and no partially applied
//! state becomes an unrecoverable orphan.

use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgConnection};
use uuid::Uuid;

use super::LibraryStore;
use super::objects::StorageObjectRecord;

/// One open or completed physical-deletion intent.
#[derive(Debug, Clone, FromRow)]
pub struct SourceObjectCleanupIntent {
    pub id: i64,
    pub object_id: Option<Uuid>,
    pub group_id: i64,
    pub sha256: Option<String>,
    pub object_key: String,
    pub storage_backend: String,
    pub attempts: i32,
    pub next_attempt_at: DateTime<Utc>,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl LibraryStore {
    /// Record one cleanup intent. Returns `None` when an open intent already
    /// exists for the object or legacy path (idempotent release).
    pub async fn insert_source_object_cleanup(
        &self,
        connection: &mut PgConnection,
        object_id: Option<Uuid>,
        group_id: i64,
        sha256: Option<&str>,
        object_key: &str,
        storage_backend: &str,
    ) -> Result<Option<i64>> {
        Ok(sqlx::query_file_scalar!(
            "src/sql/library_store/source_object_cleanup/insert_source_object_cleanup.sql",
            object_id,
            group_id,
            sha256,
            object_key,
            storage_backend
        )
        .fetch_optional(connection)
        .await?)
    }

    pub async fn lock_due_source_object_cleanups(
        &self,
        connection: &mut PgConnection,
        limit: i64,
    ) -> Result<Vec<SourceObjectCleanupIntent>> {
        Ok(sqlx::query_file_as!(
            SourceObjectCleanupIntent,
            "src/sql/library_store/source_object_cleanup/lock_due_source_object_cleanup.sql",
            limit
        )
        .fetch_all(connection)
        .await?)
    }

    pub async fn complete_source_object_cleanup(
        &self,
        connection: &mut PgConnection,
        id: i64,
        resolution: Option<&str>,
    ) -> Result<bool> {
        Ok(sqlx::query_file!(
            "src/sql/library_store/source_object_cleanup/complete_source_object_cleanup.sql",
            id,
            resolution
        )
        .fetch_optional(connection)
        .await?
        .is_some())
    }

    pub async fn reschedule_source_object_cleanup(
        &self,
        connection: &mut PgConnection,
        id: i64,
        delay_seconds: i64,
        error: &str,
    ) -> Result<()> {
        sqlx::query_file!(
            "src/sql/library_store/source_object_cleanup/reschedule_source_object_cleanup.sql",
            id,
            delay_seconds,
            error
        )
        .execute(connection)
        .await?;
        Ok(())
    }

    /// Lock and return the object row for identity validation. The row lock is
    /// held until the transaction ends and conflicts with the FOR KEY SHARE
    /// lock taken by any new file/task reference insert.
    pub async fn lock_cleanup_storage_object(
        &self,
        connection: &mut PgConnection,
        object_id: Uuid,
    ) -> Result<Option<StorageObjectRecord>> {
        Ok(sqlx::query_file_as!(
            StorageObjectRecord,
            "src/sql/library_store/source_object_cleanup/lock_cleanup_storage_object.sql",
            object_id
        )
        .fetch_optional(connection)
        .await?)
    }

    pub async fn delete_cleanup_storage_object(
        &self,
        connection: &mut PgConnection,
        object_id: Uuid,
    ) -> Result<Option<String>> {
        Ok(sqlx::query_file_scalar!(
            "src/sql/library_store/source_object_cleanup/delete_cleanup_storage_object.sql",
            object_id
        )
        .fetch_optional(connection)
        .await?)
    }

    /// Count library rows that still actively reference a legacy direct path;
    /// the deliberately released row is excluded by the query itself.
    pub async fn count_live_files_for_storage_path(
        &self,
        connection: &mut PgConnection,
        object_key: &str,
    ) -> Result<i64> {
        Ok(sqlx::query_file_scalar!(
            "src/sql/library_store/source_object_cleanup/count_live_files_for_storage_path.sql",
            object_key
        )
        .fetch_one(connection)
        .await?)
    }

    pub async fn count_live_objects_for_object_key(
        &self,
        connection: &mut PgConnection,
        object_key: &str,
    ) -> Result<i64> {
        Ok(sqlx::query_file_scalar!(
            "src/sql/library_store/source_object_cleanup/count_live_objects_for_object_key.sql",
            object_key
        )
        .fetch_one(connection)
        .await?)
    }

    pub async fn count_storage_object_references_on_connection(
        &self,
        connection: &mut PgConnection,
        object_id: Uuid,
    ) -> Result<i64> {
        #[derive(sqlx::FromRow)]
        struct ReferenceCountRow {
            references: i64,
        }
        let row = sqlx::query_file_as!(
            ReferenceCountRow,
            "src/sql/library_store/objects/count_storage_object_references.sql",
            object_id
        )
        .fetch_one(connection)
        .await?;
        Ok(row.references)
    }
}
