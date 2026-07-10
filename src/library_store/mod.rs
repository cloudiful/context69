use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

use crate::contracts::{
    LibraryDocumentSectionPreview, LibraryFileDetailResponse, LibraryFileSummary,
    LibraryIngestJobResponse, LibraryIngestStatus, LibraryPreviewContentFormat,
};
use crate::db::Database;
use crate::domain::{LibraryFileRecord, LibraryFolderRecord, LibraryIngestJobRecord};

mod detail;
mod documents;
mod files;
mod folders;
mod jobs;
mod mappers;

pub(crate) use mappers::{file_to_summary, infer_preview_content_format, job_to_response};

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
}

#[derive(Debug, Clone)]
pub struct UpdateLibraryTextFile {
    pub folder_id: Option<Uuid>,
    pub external_id: Option<String>,
    pub filename: String,
    pub media_type: String,
    pub size_bytes: i64,
    pub sha256: String,
    pub storage_rel_path: String,
}

#[derive(Debug, Clone, FromRow)]
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

#[derive(Debug, Clone, FromRow)]
struct FileRow {
    group_id: i64,
    group_key: String,
    group_path: String,
    visibility: String,
    id: Uuid,
    folder_id: Option<Uuid>,
    external_id: Option<String>,
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
struct JobRow {
    group_id: i64,
    group_key: String,
    group_path: String,
    visibility: String,
    id: Uuid,
    file_id: Uuid,
    status: String,
    docling_task_id: Option<String>,
    error_message: Option<String>,
    created_at: DateTime<Utc>,
    started_at: Option<DateTime<Utc>>,
    finished_at: Option<DateTime<Utc>>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
struct FileDocumentRow {
    file_id: Uuid,
    document_id: i64,
    group_id: i64,
    visibility: String,
    section_key: String,
    section_label: String,
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
    published_at: Option<chrono::NaiveDate>,
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
