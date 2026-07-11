use anyhow::Result;
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
}

#[derive(Debug, Clone, FromRow)]
pub struct DeletedStorageObject {
    pub object_key: String,
    pub storage_backend: String,
}

impl LibraryStore {
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
}
