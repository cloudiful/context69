use std::str::FromStr;

use anyhow::{Context, Result, anyhow};
use uuid::Uuid;

use super::{LibraryStore, ResourceRow};
use crate::contracts::{
    LibraryIngestStatus, LibraryResourceItem, LibraryResourceKind, LibraryResourceSortBy,
    SortDirection, Visibility,
};

pub struct ResourceListQuery<'a> {
    pub project_id: i64,
    pub folder_id: Option<Uuid>,
    pub query: Option<&'a str>,
    pub status: Option<LibraryIngestStatus>,
    pub sort_by: LibraryResourceSortBy,
    pub sort_direction: SortDirection,
    pub limit: i64,
    pub offset: i64,
}

impl LibraryStore {
    pub async fn count_resources_in_project_folder(
        &self,
        project_id: i64,
        folder_id: Option<Uuid>,
        query: Option<&str>,
        status: Option<LibraryIngestStatus>,
    ) -> Result<i64> {
        Ok(sqlx::query_file_scalar!(
            "src/sql/library_store/resources/count_resources_in_project_folder.sql",
            project_id,
            folder_id,
            query,
            status.map(LibraryIngestStatus::as_str)
        )
        .fetch_one(self.db.pool())
        .await?)
    }

    pub async fn list_resources_in_project_folder(
        &self,
        query: &ResourceListQuery<'_>,
    ) -> Result<Vec<LibraryResourceItem>> {
        let rows = sqlx::query_file_as!(
            ResourceRow,
            "src/sql/library_store/resources/list_resources_in_project_folder.sql",
            query.project_id,
            query.folder_id,
            query.query,
            query.status.map(LibraryIngestStatus::as_str),
            query.sort_by.as_str(),
            query.sort_direction.as_str(),
            query.limit,
            query.offset
        )
        .fetch_all(self.db.pool())
        .await?;

        rows.into_iter().map(resource_from_row).collect()
    }
}

fn resource_from_row(row: ResourceRow) -> Result<LibraryResourceItem> {
    let kind = match row.resource_kind.as_str() {
        "folder" => LibraryResourceKind::Folder,
        "file" => LibraryResourceKind::File,
        other => return Err(anyhow!("unsupported library resource kind: {other}")),
    };
    let ingest_status = row
        .ingest_status
        .as_deref()
        .map(LibraryIngestStatus::from_str)
        .transpose()?;

    Ok(LibraryResourceItem {
        kind,
        id: row.id,
        group_key: row.group_key,
        group_path: row.group_path,
        visibility: row.visibility.parse().unwrap_or(Visibility::Private),
        parent_folder_id: row.parent_folder_id,
        name: row.name,
        media_type: row.media_type,
        size_bytes: row.size_bytes,
        ingest_status,
        error_message: row.error_message,
        child_folder_count: u64::try_from(row.child_folder_count)
            .context("negative child folder count")?,
        file_count: u64::try_from(row.file_count).context("negative file count")?,
        processing_count: u64::try_from(row.processing_count)
            .context("negative processing count")?,
        is_source_folder: row.is_source_folder,
        is_source_records_folder: row.is_source_records_folder,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}
