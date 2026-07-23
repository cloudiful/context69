use anyhow::Result;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::Postgres;
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

#[derive(Debug, Clone)]
pub struct MetadataValueRow {
    pub index_id: Uuid,
    pub document_id: i64,
    pub ordinal: i32,
    pub keyword_value: Option<String>,
    pub integer_value: Option<i64>,
    pub float_value: Option<f64>,
    pub boolean_value: Option<bool>,
    pub datetime_value: Option<DateTime<Utc>>,
}

pub(crate) fn metadata_value_rows(
    index_id: Uuid,
    document_id: i64,
    values: &[crate::services::document_store::metadata::TypedMetadataValue],
) -> Vec<MetadataValueRow> {
    values
        .iter()
        .enumerate()
        .map(|(ordinal, value)| MetadataValueRow {
            index_id,
            document_id,
            ordinal: ordinal as i32,
            keyword_value: value.keyword_value.clone(),
            integer_value: value.integer_value,
            float_value: value.float_value,
            boolean_value: value.boolean_value,
            datetime_value: value.datetime_value,
        })
        .collect()
}

pub(crate) async fn replace_metadata_values_in_transaction(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    keys: &[(Uuid, i64)],
    entries: &[MetadataValueRow],
) -> Result<()> {
    if keys.is_empty() {
        return Ok(());
    }
    let index_ids = keys
        .iter()
        .map(|(index_id, _)| *index_id)
        .collect::<Vec<_>>();
    let document_ids = keys
        .iter()
        .map(|(_, document_id)| *document_id)
        .collect::<Vec<_>>();
    sqlx::query_file!(
        "src/sql/db/metadata_indexes/delete_values_bulk.sql",
        &index_ids,
        &document_ids
    )
    .execute(&mut **tx)
    .await?;

    let keyword_entries = entries
        .iter()
        .filter_map(|entry| entry.keyword_value.as_ref().map(|value| (entry, value)))
        .collect::<Vec<_>>();
    if !keyword_entries.is_empty() {
        let index_ids = keyword_entries
            .iter()
            .map(|(entry, _)| entry.index_id)
            .collect::<Vec<_>>();
        let document_ids = keyword_entries
            .iter()
            .map(|(entry, _)| entry.document_id)
            .collect::<Vec<_>>();
        let ordinals = keyword_entries
            .iter()
            .map(|(entry, _)| entry.ordinal)
            .collect::<Vec<_>>();
        let values = keyword_entries
            .iter()
            .map(|(_, value)| (*value).clone())
            .collect::<Vec<_>>();
        sqlx::query_file!(
            "src/sql/db/metadata_indexes/insert_keyword_values_bulk.sql",
            &index_ids,
            &document_ids,
            &ordinals,
            &values
        )
        .execute(&mut **tx)
        .await?;
    }

    macro_rules! insert_typed_values {
        ($field:ident, $query_file:literal, $value_type:ty) => {{
            let typed_entries = entries
                .iter()
                .filter_map(|entry| entry.$field.map(|value| (entry, value)))
                .collect::<Vec<_>>();
            if !typed_entries.is_empty() {
                let index_ids = typed_entries
                    .iter()
                    .map(|(entry, _)| entry.index_id)
                    .collect::<Vec<_>>();
                let document_ids = typed_entries
                    .iter()
                    .map(|(entry, _)| entry.document_id)
                    .collect::<Vec<_>>();
                let ordinals = typed_entries
                    .iter()
                    .map(|(entry, _)| entry.ordinal)
                    .collect::<Vec<_>>();
                let values = typed_entries
                    .iter()
                    .map(|(_, value)| *value)
                    .collect::<Vec<$value_type>>();
                sqlx::query_file!($query_file, &index_ids, &document_ids, &ordinals, &values)
                    .execute(&mut **tx)
                    .await?;
            }
        }};
    }
    insert_typed_values!(
        integer_value,
        "src/sql/db/metadata_indexes/insert_integer_values_bulk.sql",
        i64
    );
    insert_typed_values!(
        float_value,
        "src/sql/db/metadata_indexes/insert_float_values_bulk.sql",
        f64
    );
    insert_typed_values!(
        boolean_value,
        "src/sql/db/metadata_indexes/insert_boolean_values_bulk.sql",
        bool
    );
    insert_typed_values!(
        datetime_value,
        "src/sql/db/metadata_indexes/insert_datetime_values_bulk.sql",
        DateTime<Utc>
    );
    Ok(())
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
        let entries = metadata_value_rows(index_id, document_id, values);
        self.replace_metadata_values_bulk(&[(index_id, document_id)], &entries)
            .await
    }

    pub async fn replace_metadata_values_bulk(
        &self,
        keys: &[(Uuid, i64)],
        entries: &[MetadataValueRow],
    ) -> Result<()> {
        if keys.is_empty() {
            return Ok(());
        }
        let mut tx = self.pool.begin().await?;
        replace_metadata_values_in_transaction(&mut tx, keys, entries).await?;
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
