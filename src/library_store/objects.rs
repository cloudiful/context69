use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

use super::LibraryStore;

#[derive(Debug, Clone, FromRow)]
pub struct StorageObjectRecord {
    pub id: Uuid,
    pub group_id: i64,
    pub sha256: String,
    pub size_bytes: i64,
    pub storage_backend: String,
    pub object_key: String,
    pub staging_lease_until: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, FromRow)]
pub struct DeletedStorageObject {
    pub object_key: String,
    pub storage_backend: String,
}

impl LibraryStore {
    pub async fn get_storage_object_on_connection(
        &self,
        connection: &mut sqlx::PgConnection,
        group_id: i64,
        sha256: &str,
    ) -> Result<Option<StorageObjectRecord>> {
        Ok(sqlx::query_file_as!(
            StorageObjectRecord,
            "src/sql/library_store/objects/get_storage_object.sql",
            group_id,
            sha256
        )
        .fetch_optional(connection)
        .await?)
    }

    pub async fn upsert_staged_storage_object_on_connection(
        &self,
        connection: &mut sqlx::PgConnection,
        id: Uuid,
        group_id: i64,
        sha256: &str,
        size_bytes: i64,
        storage_backend: &str,
        object_key: &str,
        staging_lease_until: DateTime<Utc>,
    ) -> Result<StorageObjectRecord> {
        Ok(sqlx::query_file_as!(
            StorageObjectRecord,
            "src/sql/library_store/objects/upsert_staged_storage_object.sql",
            id,
            group_id,
            sha256,
            size_bytes,
            storage_backend,
            object_key,
            staging_lease_until
        )
        .fetch_one(connection)
        .await?)
    }

    pub async fn upsert_storage_object_on_connection(
        &self,
        connection: &mut sqlx::PgConnection,
        id: Uuid,
        group_id: i64,
        sha256: &str,
        size_bytes: i64,
        storage_backend: &str,
        object_key: &str,
    ) -> Result<StorageObjectRecord> {
        Ok(sqlx::query_file_as!(
            StorageObjectRecord,
            "src/sql/library_store/objects/upsert_storage_object.sql",
            id,
            group_id,
            sha256,
            size_bytes,
            storage_backend,
            object_key
        )
        .fetch_one(connection)
        .await?)
    }

    pub async fn link_file_storage_object(
        &self,
        file_id: Uuid,
        object_id: Uuid,
        object_key: &str,
    ) -> Result<()> {
        sqlx::query_file!(
            "src/sql/library_store/objects/link_file_storage_object.sql",
            file_id,
            object_id,
            object_key
        )
        .execute(self.db.pool())
        .await?;
        Ok(())
    }

    pub async fn get_storage_object(
        &self,
        group_id: i64,
        sha256: &str,
    ) -> Result<Option<StorageObjectRecord>> {
        Ok(sqlx::query_file_as!(
            StorageObjectRecord,
            "src/sql/library_store/objects/get_storage_object.sql",
            group_id,
            sha256
        )
        .fetch_optional(self.db.pool())
        .await?)
    }

    pub async fn get_storage_object_by_id(
        &self,
        object_id: Uuid,
    ) -> Result<Option<StorageObjectRecord>> {
        Ok(sqlx::query_file_as!(
            StorageObjectRecord,
            "src/sql/library_store/objects/get_storage_object_by_id.sql",
            object_id
        )
        .fetch_optional(self.db.pool())
        .await?)
    }

    pub async fn get_storage_object_by_id_on_connection(
        &self,
        connection: &mut sqlx::PgConnection,
        object_id: Uuid,
    ) -> Result<Option<StorageObjectRecord>> {
        Ok(sqlx::query_file_as!(
            StorageObjectRecord,
            "src/sql/library_store/objects/get_storage_object_by_id_on_connection.sql",
            object_id
        )
        .fetch_optional(connection)
        .await?)
    }

    pub async fn lock_storage_object(
        &self,
        connection: &mut sqlx::PgConnection,
        lock_key: &str,
    ) -> Result<()> {
        sqlx::query_file!(
            "src/sql/library_store/objects/lock_storage_object.sql",
            lock_key
        )
        .execute(connection)
        .await?;
        Ok(())
    }

    pub async fn get_storage_object_by_id_for_update(
        &self,
        connection: &mut sqlx::PgConnection,
        object_id: Uuid,
        before: DateTime<Utc>,
    ) -> Result<Option<StorageObjectRecord>> {
        Ok(sqlx::query_file_as!(
            StorageObjectRecord,
            "src/sql/library_store/objects/get_storage_object_by_id_for_update.sql",
            object_id,
            before
        )
        .fetch_optional(connection)
        .await?)
    }

    pub async fn upsert_storage_object(
        &self,
        id: Uuid,
        group_id: i64,
        sha256: &str,
        size_bytes: i64,
        storage_backend: &str,
        object_key: &str,
    ) -> Result<StorageObjectRecord> {
        Ok(sqlx::query_file_as!(
            StorageObjectRecord,
            "src/sql/library_store/objects/upsert_storage_object.sql",
            id,
            group_id,
            sha256,
            size_bytes,
            storage_backend,
            object_key
        )
        .fetch_one(self.db.pool())
        .await?)
    }

    pub async fn delete_unreferenced_storage_object(
        &self,
        id: Uuid,
    ) -> Result<Option<DeletedStorageObject>> {
        Ok(sqlx::query_file_as!(
            DeletedStorageObject,
            "src/sql/library_store/objects/delete_unreferenced_storage_object.sql",
            id
        )
        .fetch_optional(self.db.pool())
        .await?)
    }

    pub async fn clear_storage_object_staged(&self, object_id: Uuid, file_id: Uuid) -> Result<()> {
        sqlx::query_file!(
            "src/sql/library_store/objects/clear_staged_storage_object.sql",
            object_id,
            file_id
        )
        .execute(self.db.pool())
        .await?;
        Ok(())
    }

    pub async fn get_staged_storage_object_for_update(
        &self,
        connection: &mut sqlx::PgConnection,
        object_id: Uuid,
    ) -> Result<Option<StorageObjectRecord>> {
        Ok(sqlx::query_file_as!(
            StorageObjectRecord,
            "src/sql/library_store/objects/get_staged_storage_object_for_update.sql",
            object_id
        )
        .fetch_optional(connection)
        .await?)
    }

    pub async fn delete_released_staged_storage_object(
        &self,
        connection: &mut sqlx::PgConnection,
        object_id: Uuid,
    ) -> Result<bool> {
        Ok(sqlx::query_file!(
            "src/sql/library_store/objects/delete_released_staged_storage_object.sql",
            object_id
        )
        .fetch_optional(connection)
        .await?
        .is_some())
    }

    pub async fn clear_expired_staging_with_file_reference(
        &self,
        before: DateTime<Utc>,
        limit: i64,
    ) -> Result<u64> {
        Ok(sqlx::query_file!(
            "src/sql/library_store/objects/clear_expired_staging_with_file_reference.sql",
            before,
            limit
        )
        .execute(self.db.pool())
        .await?
        .rows_affected())
    }

    pub async fn sweep_orphaned_storage_objects(
        &self,
        before: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<StorageObjectRecord>> {
        Ok(sqlx::query_file_as!(
            StorageObjectRecord,
            "src/sql/library_store/objects/list_orphaned_storage_objects.sql",
            before,
            limit
        )
        .fetch_all(self.db.pool())
        .await?)
    }

    pub async fn delete_orphaned_storage_object_record_for_update(
        &self,
        connection: &mut sqlx::PgConnection,
        object_id: Uuid,
        before: DateTime<Utc>,
    ) -> Result<bool> {
        Ok(sqlx::query_file!(
            "src/sql/library_store/objects/delete_orphaned_storage_object_record_for_update.sql",
            object_id,
            before
        )
        .fetch_optional(connection)
        .await?
        .is_some())
    }
}
