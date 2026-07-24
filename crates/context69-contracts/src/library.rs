use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use super::Visibility;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LibraryIngestStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
}

impl LibraryIngestStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
        }
    }
}

impl std::str::FromStr for LibraryIngestStatus {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "pending" => Ok(Self::Pending),
            "running" => Ok(Self::Running),
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            other => Err(anyhow::anyhow!(
                "unsupported library ingest status: {other}"
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LibraryIngestFailureStage {
    Download,
    Storage,
    Docling,
    Parsing,
    Embedding,
    Indexing,
    Translation,
    Other,
}

impl LibraryIngestFailureStage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Download => "download",
            Self::Storage => "storage",
            Self::Docling => "docling",
            Self::Parsing => "parsing",
            Self::Embedding => "embedding",
            Self::Indexing => "indexing",
            Self::Translation => "translation",
            Self::Other => "other",
        }
    }
}

impl std::str::FromStr for LibraryIngestFailureStage {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "download" => Ok(Self::Download),
            "storage" => Ok(Self::Storage),
            "docling" => Ok(Self::Docling),
            "parsing" => Ok(Self::Parsing),
            "embedding" => Ok(Self::Embedding),
            "indexing" => Ok(Self::Indexing),
            "translation" => Ok(Self::Translation),
            "other" => Ok(Self::Other),
            other => Err(anyhow::anyhow!(
                "unsupported library ingest failure stage: {other}"
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateFolderRequest {
    #[serde(default)]
    pub parent_folder_id: Option<Uuid>,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MoveFolderRequest {
    #[serde(default)]
    pub target_folder_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MoveFileRequest {
    #[serde(default)]
    pub target_folder_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateTextRequest {
    #[serde(default)]
    pub folder_id: Option<Uuid>,
    pub title: String,
    pub content: String,
    #[serde(default = "default_text_content_format")]
    pub content_format: LibraryTextContentFormat,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub translation: Option<crate::TranslationDirective>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpsertLibraryTextRequest {
    pub external_id: String,
    #[serde(default)]
    pub folder_id: Option<Uuid>,
    pub title: String,
    pub content: String,
    #[serde(default = "default_text_content_format")]
    pub content_format: LibraryTextContentFormat,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_at: Option<DateTime<Utc>>,
    #[serde(default = "default_metadata_json")]
    #[schema(value_type = Object)]
    pub metadata_json: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub translation: Option<crate::TranslationDirective>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LibraryFileSummary {
    pub file_id: Uuid,
    pub group_key: String,
    pub group_path: String,
    pub visibility: Visibility,
    #[serde(default)]
    pub folder_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_at: Option<DateTime<Utc>>,
    #[serde(default = "default_metadata_json")]
    #[schema(value_type = Object)]
    pub metadata_json: Value,
    pub filename: String,
    pub media_type: String,
    pub size_bytes: i64,
    pub ingest_status: LibraryIngestStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ingested_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LibraryFolderNode {
    pub group_key: String,
    pub group_path: String,
    pub visibility: Visibility,
    #[serde(default)]
    pub folder_id: Option<Uuid>,
    #[serde(default)]
    pub parent_folder_id: Option<Uuid>,
    pub name: String,
    pub path: String,
    pub processing_count: usize,
    #[schema(no_recursion)]
    pub children: Vec<LibraryFolderNode>,
    pub files: Vec<LibraryFileSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LibraryFolderResponse {
    pub folder_id: Uuid,
    pub group_key: String,
    pub group_path: String,
    pub visibility: Visibility,
    #[serde(default)]
    pub parent_folder_id: Option<Uuid>,
    pub name: String,
    pub path: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LibraryTreeResponse {
    pub root: LibraryFolderNode,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LibraryResourceKind {
    Folder,
    File,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LibraryResourceSortBy {
    Name,
    Type,
    Status,
    Size,
    UpdatedAt,
}

impl LibraryResourceSortBy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Type => "type",
            Self::Status => "status",
            Self::Size => "size",
            Self::UpdatedAt => "updated_at",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SortDirection {
    Asc,
    Desc,
}

impl SortDirection {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Asc => "asc",
            Self::Desc => "desc",
        }
    }
}

fn default_page() -> u32 {
    1
}

fn default_page_size() -> u32 {
    50
}

fn default_resource_sort_by() -> LibraryResourceSortBy {
    LibraryResourceSortBy::UpdatedAt
}

fn default_sort_direction() -> SortDirection {
    SortDirection::Desc
}

#[derive(Debug, Clone, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct LibraryResourcePageQuery {
    #[serde(default)]
    pub folder_id: Option<Uuid>,
    #[serde(default = "default_page")]
    pub page: u32,
    #[serde(default = "default_page_size")]
    pub page_size: u32,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub status: Option<LibraryIngestStatus>,
    #[serde(default = "default_resource_sort_by")]
    pub sort_by: LibraryResourceSortBy,
    #[serde(default = "default_sort_direction")]
    pub sort_direction: SortDirection,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LibraryResourceItem {
    pub kind: LibraryResourceKind,
    pub id: Uuid,
    pub group_key: String,
    pub group_path: String,
    pub visibility: Visibility,
    #[serde(default)]
    pub parent_folder_id: Option<Uuid>,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ingest_status: Option<LibraryIngestStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub child_folder_count: u64,
    pub file_count: u64,
    pub processing_count: u64,
    pub is_source_folder: bool,
    pub is_source_records_folder: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LibraryResourcePageResponse {
    pub items: Vec<LibraryResourceItem>,
    pub pagination: crate::Pagination,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LibraryIngestJobResponse {
    pub job_id: Uuid,
    pub group_key: String,
    pub group_path: String,
    pub visibility: Visibility,
    pub file_id: Uuid,
    pub status: LibraryIngestStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docling_task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_stage: Option<LibraryIngestFailureStage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LibraryProcessingJobKind {
    Ingest,
    UrlImport,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LibraryProcessingJobResponse {
    pub job_id: Uuid,
    pub kind: LibraryProcessingJobKind,
    pub group_key: String,
    pub group_path: String,
    pub visibility: Visibility,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    pub status: LibraryIngestStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_stage: Option<LibraryIngestFailureStage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub can_retry: bool,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct LibraryProcessingJobPageQuery {
    #[serde(default = "default_page")]
    pub page: u32,
    #[serde(default = "default_page_size")]
    pub page_size: u32,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub status: Option<LibraryIngestStatus>,
    #[serde(default)]
    pub failure_stage: Option<LibraryIngestFailureStage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LibraryProcessingJobPageResponse {
    pub items: Vec<LibraryProcessingJobResponse>,
    pub pagination: crate::Pagination,
    pub summary: LibraryProcessingJobSummaryResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LibraryProcessingJobSummaryResponse {
    pub can_manage: bool,
    pub pending_count: u64,
    pub running_count: u64,
    pub failed_count: u64,
    pub stuck_count: u64,
    pub retryable_failed_count: u64,
    pub cleanupable_stuck_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LibraryProcessingJobBulkActionResponse {
    pub accepted: u64,
    pub skipped: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LibraryTextContentFormat {
    PlainText,
    Markdown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LibraryPreviewContentFormat {
    PlainText,
    Markdown,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LibraryDocumentSectionPreview {
    pub document_id: i64,
    pub section_key: String,
    pub section_label: String,
    pub sort_order: i32,
    pub title: String,
    pub preview_text: String,
    #[serde(default = "default_preview_content_format")]
    pub content_format: LibraryPreviewContentFormat,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LibraryFileDetailResponse {
    pub file_id: Uuid,
    pub group_key: String,
    pub group_path: String,
    pub visibility: Visibility,
    #[serde(default)]
    pub folder_id: Option<Uuid>,
    pub folder_path: String,
    pub filename: String,
    pub media_type: String,
    pub size_bytes: i64,
    pub sha256: String,
    #[serde(default)]
    pub source_available: bool,
    pub ingest_status: LibraryIngestStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ingested_at: Option<DateTime<Utc>>,
    pub sections: Vec<LibraryDocumentSectionPreview>,
    pub jobs: Vec<LibraryIngestJobResponse>,
}

#[derive(Debug, Clone, Deserialize, IntoParams, ToSchema)]
#[into_params(parameter_in = Query)]
pub struct LibraryFileJobPageQuery {
    #[serde(default = "default_page")]
    pub page: u32,
    #[serde(default = "default_page_size")]
    pub page_size: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LibraryFileJobPageResponse {
    pub items: Vec<LibraryIngestJobResponse>,
    pub pagination: crate::Pagination,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LibraryUploadResponse {
    pub files: Vec<LibraryFileSummary>,
    pub jobs: Vec<LibraryIngestJobResponse>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct LibraryFileUploadMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_at: Option<DateTime<Utc>>,
    #[serde(default = "default_metadata_json")]
    #[schema(value_type = Object)]
    pub metadata_json: Value,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct LibraryFileIngestOptions {
    #[serde(flatten)]
    pub metadata: LibraryFileUploadMetadata,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub translation: Option<crate::TranslationDirective>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PrepareLibraryUploadRequest {
    #[serde(default)]
    pub folder_id: Option<Uuid>,
    pub filename: String,
    pub media_type: String,
    pub size_bytes: i64,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<LibraryFileUploadMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub translation: Option<crate::TranslationDirective>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PrepareLibraryUploadResponse {
    pub upload_required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<LibraryFileSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<LibraryIngestJobResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ImportLibraryFileFromUrlRequest {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<LibraryFileUploadMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub translation: Option<crate::TranslationDirective>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LibraryUrlImportStatus {
    Queued,
    Downloading,
    Ingesting,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LibraryUrlImportJobResponse {
    pub import_job_id: Uuid,
    pub group_path: String,
    pub source_url: String,
    pub status: LibraryUrlImportStatus,
    pub attempt_count: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<LibraryFileSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ingest_job: Option<LibraryIngestJobResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_stage: Option<LibraryIngestFailureStage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

fn default_preview_content_format() -> LibraryPreviewContentFormat {
    LibraryPreviewContentFormat::PlainText
}

fn default_text_content_format() -> LibraryTextContentFormat {
    LibraryTextContentFormat::PlainText
}

fn default_metadata_json() -> Value {
    json!({})
}
