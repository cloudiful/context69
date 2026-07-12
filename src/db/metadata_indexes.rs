use anyhow::Result;
use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

use super::Database;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct StoredMetadataIndex {
    pub index_id: Uuid,
    pub group_id: i64,
    pub group_path: String,
    pub source_key: String,
    pub field_path: String,
    pub data_type: String,
    pub value_kind: String,
    pub sortable: bool,
    pub status: String,
    pub processed_documents: i64,
    pub total_documents: i64,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct MetadataDocument {
    pub document_id: i64,
    pub metadata_json: Value,
}

pub struct NewMetadataIndex<'a> {
    pub index_id: Uuid,
    pub group_id: i64,
    pub source_key: &'a str,
    pub field_path: &'a str,
    pub data_type: &'a str,
    pub value_kind: &'a str,
    pub sortable: bool,
}

impl Database {
    pub async fn list_metadata_indexes(
        &self,
        group_id: i64,
        source_key: &str,
    ) -> Result<Vec<StoredMetadataIndex>> {
        Ok(sqlx::query_file_as!(
            StoredMetadataIndex,
            "src/sql/db/metadata_indexes/list.sql",
            group_id,
            source_key
        )
        .fetch_all(self.pool())
        .await?)
    }

    pub async fn get_metadata_index(&self, index_id: Uuid) -> Result<Option<StoredMetadataIndex>> {
        Ok(sqlx::query_file_as!(
            StoredMetadataIndex,
            "src/sql/db/metadata_indexes/get.sql",
            index_id
        )
        .fetch_optional(self.pool())
        .await?)
    }

    pub async fn create_metadata_index(
        &self,
        command: &NewMetadataIndex<'_>,
    ) -> Result<StoredMetadataIndex> {
        let total = sqlx::query_file_scalar!(
            "src/sql/db/metadata_indexes/count_documents.sql",
            command.group_id,
            command.source_key
        )
        .fetch_one(self.pool())
        .await?
        .unwrap_or(0);
        sqlx::query_file!(
            "src/sql/db/metadata_indexes/create.sql",
            command.index_id,
            command.group_id,
            command.source_key,
            command.field_path,
            command.data_type,
            command.value_kind,
            command.sortable,
            total
        )
        .execute(self.pool())
        .await?;
        Ok(self
            .get_metadata_index(command.index_id)
            .await?
            .expect("created index"))
    }

    pub async fn mark_metadata_index_building(
        &self,
        index_id: Uuid,
        data_type: &str,
        value_kind: &str,
        sortable: bool,
    ) -> Result<()> {
        sqlx::query_file!(
            "src/sql/db/metadata_indexes/mark_building.sql",
            index_id,
            data_type,
            value_kind,
            sortable
        )
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn mark_metadata_index_deleting(&self, index_id: Uuid) -> Result<()> {
        sqlx::query_file!("src/sql/db/metadata_indexes/mark_deleting.sql", index_id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn pending_metadata_indexes(&self) -> Result<Vec<StoredMetadataIndex>> {
        Ok(sqlx::query_file_as!(
            StoredMetadataIndex,
            "src/sql/db/metadata_indexes/list_pending.sql"
        )
        .fetch_all(self.pool())
        .await?)
    }

    pub async fn metadata_documents(
        &self,
        index: &StoredMetadataIndex,
    ) -> Result<Vec<MetadataDocument>> {
        Ok(sqlx::query_file_as!(
            MetadataDocument,
            "src/sql/db/metadata_indexes/list_documents.sql",
            index.group_id,
            index.source_key
        )
        .fetch_all(self.pool())
        .await?)
    }

    pub async fn replace_metadata_values(
        &self,
        index_id: Uuid,
        document_id: i64,
        values: &[crate::services::document_store::metadata::TypedMetadataValue],
    ) -> Result<()> {
        let mut tx = self.pool().begin().await?;
        sqlx::query_file!(
            "src/sql/db/metadata_indexes/delete_document_values.sql",
            index_id,
            document_id
        )
        .execute(&mut *tx)
        .await?;
        for (ordinal, value) in values.iter().enumerate() {
            sqlx::query_file!(
                "src/sql/db/metadata_indexes/insert_value.sql",
                index_id,
                document_id,
                ordinal as i32,
                value.keyword_value.as_deref(),
                value.integer_value,
                value.float_value,
                value.boolean_value,
                value.datetime_value
            )
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn finish_metadata_index(&self, index_id: Uuid, processed: i64) -> Result<()> {
        sqlx::query_file!(
            "src/sql/db/metadata_indexes/finish.sql",
            index_id,
            processed
        )
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn fail_metadata_index(&self, index_id: Uuid, error: &str) -> Result<()> {
        sqlx::query_file!("src/sql/db/metadata_indexes/fail.sql", index_id, error)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    pub async fn remove_metadata_index(&self, index_id: Uuid) -> Result<()> {
        sqlx::query_file!("src/sql/db/metadata_indexes/delete.sql", index_id)
            .execute(self.pool())
            .await?;
        Ok(())
    }
}
