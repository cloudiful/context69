use chrono::{DateTime, Utc};
use sea_orm::sea_query::{
    Alias, ColumnRef, CommonTableExpression, Expr as QueryExpr, ExprTrait as QueryExprTrait,
    Query as QuerySelectBuilder, SelectStatement, SubQueryStatement, TableRef, UpdateStatement,
    WithClause,
};
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

use crate::contracts::{
    LibraryDocumentSectionPreview, LibraryFileDetailResponse, LibraryFileSummary,
    LibraryIngestStatus, LibraryPreviewContentFormat,
};
use crate::db::Database;
use crate::db::entity::groups;
use crate::domain::{LibraryFileRecord, LibraryFolderRecord};

mod dependency_gates;
mod detail;
pub(crate) mod documents;
mod files;
mod folders;
mod group_settings;
mod mappers;
pub(crate) mod objects;
mod resources;
mod source_object_cleanup;
mod source_release;
mod task_write_fence;
pub use resources::ResourceListQuery;
pub use source_object_cleanup::SourceObjectCleanupIntent;
pub use source_release::{FileSourceLifecycleRow, PendingAutoReleaseFile};
pub(crate) use task_write_fence::begin_task_write_with_lease;
pub use task_write_fence::{
    TaskWriteDenied, TaskWriteFence, TaskWriteTx, begin_task_write, task_write_granted,
    write_task_item_payload,
};

pub use dependency_gates::{DependencyGateRecord, DependencyGateTransition};
pub(crate) use files::LegacyDirectPathFileRow;
pub use files::MissingLegacySourceFileRow;
pub(crate) use mappers::{file_to_summary, infer_preview_content_format};

#[derive(Clone)]
pub struct LibraryStore {
    db: Database,
}

#[derive(Debug, Clone)]
pub struct NewLibraryFile {
    pub id: Uuid,
    pub folder_id: Option<Uuid>,
    pub external_id: Option<String>,
    pub filename: String,
    pub media_type: String,
    pub size_bytes: i64,
    pub sha256: String,
    pub storage_rel_path: String,
    pub storage_object_id: Option<Uuid>,
    pub delete_source_after_processing: bool,
}

#[derive(Debug, Clone)]
pub struct UpdateLibraryFileContent {
    pub folder_id: Option<Uuid>,
    pub external_id: Option<String>,
    pub filename: String,
    pub media_type: String,
    pub size_bytes: i64,
    pub sha256: String,
    pub storage_rel_path: String,
    pub storage_object_id: Option<Uuid>,
}

#[derive(Debug, Clone, FromRow, sea_orm::FromQueryResult)]
struct FolderRow {
    group_id: i64,
    group_key: String,
    group_path: String,
    visibility: String,
    id: Uuid,
    parent_id: Option<Uuid>,
    name: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, sea_orm::FromQueryResult)]
struct FileRow {
    group_id: i64,
    group_key: String,
    group_path: String,
    visibility: String,
    id: Uuid,
    folder_id: Option<Uuid>,
    external_id: Option<String>,
    source_uri: Option<String>,
    published_at: Option<DateTime<Utc>>,
    metadata_json: Value,
    filename: String,
    media_type: String,
    size_bytes: i64,
    sha256: String,
    storage_rel_path: String,
    ingest_status: String,
    error_message: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    ingested_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, FromRow)]
struct ResourceRow {
    resource_kind: String,
    id: Uuid,
    group_key: String,
    group_path: String,
    visibility: String,
    parent_folder_id: Option<Uuid>,
    name: String,
    media_type: Option<String>,
    size_bytes: Option<i64>,
    ingest_status: Option<String>,
    error_message: Option<String>,
    child_folder_count: i64,
    file_count: i64,
    processing_count: i64,
    is_source_folder: bool,
    is_source_records_folder: bool,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, sea_orm::FromQueryResult)]
struct FileDocumentRow {
    file_id: Uuid,
    document_id: i64,
    group_id: i64,
    visibility: String,
    section_key: String,
    section_label: String,
    section_external_id: Option<String>,
    section_source_uri: Option<String>,
    section_published_at: Option<DateTime<Utc>>,
    section_metadata_json: Value,
    sort_order: i32,
}

#[derive(Debug, Clone, FromRow)]
struct ChunkPayloadRow {
    chunk_id: Uuid,
    document_id: i64,
    group_id: i64,
    group_key: String,
    group_path: String,
    visibility: String,
    source_key: String,
    external_id: String,
    title: String,
    summary: Option<String>,
    source_uri: String,
    published_at: Option<chrono::DateTime<chrono::Utc>>,
    updated_at_source: DateTime<Utc>,
    record_hash: String,
    chunk_index: i32,
    chunk_text: String,
    metadata_json: Value,
}

#[derive(Debug, Clone, FromRow)]
struct FileDetailRow {
    group_key: String,
    group_path: String,
    visibility: String,
    file_id: Uuid,
    folder_id: Option<Uuid>,
    filename: String,
    media_type: String,
    size_bytes: i64,
    sha256: String,
    ingest_status: String,
    error_message: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    ingested_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, FromRow)]
struct SectionPreviewRow {
    document_id: i64,
    section_key: String,
    section_label: String,
    sort_order: i32,
    title: String,
    media_type: String,
    chunk_text: Option<String>,
}

impl LibraryStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

/// The derived group projection of one library row.
///
/// The group key and the group path are not stored on the library rows: they
/// are read from `context69.groups`, which keeps its own owner. The
/// projection is a correlated scalar subquery over the owning group, so the
/// native reads return exactly the same two columns as the previous
/// statements.
///
/// The projected column is a typed group column, so the projection can only
/// name a column the group table really has. The group path is the
/// `full_path` of the group, and the caller keeps the `group_path` name of
/// the response.
fn group_projection(column: groups::Column, owner_group_id: ColumnRef) -> QueryExpr {
    let groups = TableRef::from((Alias::new("context69"), Alias::new("groups")));
    let inner = QuerySelectBuilder::select()
        .column(ColumnRef::from((Alias::new("groups"), column)))
        .and_where(
            QueryExpr::col(ColumnRef::from((Alias::new("groups"), groups::Column::Id)))
                .eq(QueryExpr::col(owner_group_id)),
        )
        .from(groups)
        .to_owned();
    QueryExpr::from(SubQueryStatement::from(inner))
}

/// The statement that updates library rows and projects the rows it updated.
///
/// The update and the projection of the updated rows are one statement, so
/// the projected row is the row of the update statement's own snapshot: no
/// later statement can observe a newer group path than the update did.
pub(crate) fn update_with_projection(
    update: UpdateStatement,
    cte_name: &str,
    projection: SelectStatement,
) -> sea_orm::sea_query::WithQuery {
    let mut cte = CommonTableExpression::new();
    cte.table_name(Alias::new(cte_name));
    cte.query(update);
    let mut with = WithClause::new();
    with.cte(cte);
    with.query(projection)
}
