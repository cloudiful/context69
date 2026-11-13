//! Store access for the library source-file lifecycle (issue 332 phase 2).
//!
//! `delete_source_after_processing` and `source_released_at` are deliberately
//! kept off [`super::FileRow`] so the hot file-listing queries stay unchanged;
//! the release path reads them through the small, focused queries here.

use anyhow::Result;
use chrono::{DateTime, Utc};
use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use sqlx::{FromRow, PgConnection};
use uuid::Uuid;

use super::LibraryStore;
use crate::db::entity::library_files;

/// Source-file lifecycle state of one `library_files` row.
#[derive(Debug, Clone, FromRow, sea_orm::FromQueryResult)]
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
#[derive(Debug, Clone, FromRow, sea_orm::FromQueryResult)]
pub struct PendingAutoReleaseFile {
    pub id: Uuid,
    pub group_id: i64,
}

impl LibraryStore {
    pub async fn get_file_source_lifecycle(
        &self,
        file_id: Uuid,
    ) -> Result<Option<FileSourceLifecycleRow>> {
        let row = library_files::Entity::find_by_id(file_id)
            .select_only()
            .column(library_files::Column::Id)
            .column(library_files::Column::GroupId)
            .column(library_files::Column::Filename)
            .column(library_files::Column::ExternalId)
            .column(library_files::Column::IngestStatus)
            .column(library_files::Column::StorageRelPath)
            .column(library_files::Column::StorageObjectId)
            .column(library_files::Column::Sha256)
            .column(library_files::Column::SizeBytes)
            .column(library_files::Column::DeleteSourceAfterProcessing)
            .column(library_files::Column::SourceReleasedAt)
            .into_model::<FileSourceLifecycleRow>()
            .one(self.db.orm())
            .await?;
        Ok(row)
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
        let managed_source = Condition::any()
            .add(library_files::Column::ExternalId.is_null())
            .add(sea_orm::sea_query::Expr::cust(
                "external_id NOT LIKE 'source-folder:%'",
            ));
        let rows = library_files::Entity::find()
            .select_only()
            .column(library_files::Column::Id)
            .column(library_files::Column::GroupId)
            .filter(library_files::Column::DeleteSourceAfterProcessing.eq(true))
            .filter(library_files::Column::SourceReleasedAt.is_null())
            .filter(library_files::Column::IngestStatus.eq("succeeded"))
            .filter(sea_orm::sea_query::Expr::cust(
                "lower(filename) <> 'source.json'",
            ))
            .filter(managed_source)
            .order_by_asc(library_files::Column::Id)
            .limit(
                u64::try_from(limit)
                    .map_err(|_| anyhow::anyhow!("the page size must not be negative"))?,
            )
            .into_model::<PendingAutoReleaseFile>()
            .all(self.db.orm())
            .await?;
        Ok(rows)
    }
}
