use anyhow::Result;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use context69_contracts_core::common::Pagination;
use context69_contracts_core::{TaskRef, Visibility};

pub use context69_contracts_core::common::{
    LibraryDependencyGateResponse, LibraryProcessingMetric, LibraryProcessingQueueHealth,
};
pub use context69_contracts_core::pagination::SortDirection;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LibraryIngestStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl LibraryIngestStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
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
            "cancelled" => Ok(Self::Cancelled),
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extraction: Option<crate::ExtractionDirective>,
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

fn default_page() -> u32 {
    context69_contracts_core::pagination::default_page()
}

fn default_page_size() -> u32 {
    context69_contracts_core::pagination::default_page_size()
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
    pub pagination: Pagination,
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
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema, JsonSchema)]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extraction: Option<crate::ExtractionDirective>,
    /// Release the source object once processing succeeds. Chosen once at
    /// upload; defaults to `false` (retain the source).
    /// Deprecated: use [`crate::SourcePolicy`] via [`crate::IngestOptions`]
    /// instead. The multipart `metadata` field also accepts a bare
    /// [`crate::IngestOptions`] document (`metadata` object plus
    /// `source_policy`) for the v0.16 wire.
    #[serde(default)]
    pub delete_source_after_processing: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PrepareLibraryUploadRequest {
    #[serde(default)]
    pub folder_id: Option<Uuid>,
    pub filename: String,
    pub media_type: String,
    pub size_bytes: i64,
    pub sha256: String,
    /// Canonical ingest options. When present, takes precedence over the
    /// deprecated flattened fields below. New clients should send only this;
    /// v0.15 clients send only the flattened fields and stay readable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<crate::IngestOptions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<LibraryFileUploadMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub translation: Option<crate::TranslationDirective>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extraction: Option<crate::ExtractionDirective>,
    /// Release the source object once processing succeeds. Chosen once at
    /// upload; defaults to `false` (retain the source).
    /// Deprecated: use `options.source_policy` instead.
    #[serde(default)]
    pub delete_source_after_processing: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PrepareLibraryUploadResponse {
    pub upload_required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<LibraryFileSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<TaskRef>,
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
    /// Canonical ingest options. When present, takes precedence over the
    /// deprecated flattened fields below. New clients should send only this;
    /// v0.15 clients send only the flattened fields and stay readable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<crate::IngestOptions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<LibraryFileUploadMetadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub translation: Option<crate::TranslationDirective>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extraction: Option<crate::ExtractionDirective>,
    /// Release the source object once processing succeeds. Chosen once at
    /// upload; defaults to `false` (retain the source).
    /// Deprecated: use `options.source_policy` instead.
    #[serde(default)]
    pub delete_source_after_processing: bool,
}

fn default_preview_content_format() -> LibraryPreviewContentFormat {
    LibraryPreviewContentFormat::PlainText
}

fn default_text_content_format() -> LibraryTextContentFormat {
    LibraryTextContentFormat::PlainText
}

fn default_metadata_json() -> Value {
    context69_contracts_core::common::default_metadata_json()
}

/// Canonical upload shapes share [`crate::IngestOptions`]; the flattened
/// `metadata`/`translation`/`extraction` plus
/// `delete_source_after_processing` fields below stay for v0.15 wire
/// compatibility.
///
/// Deprecated: `delete_source_after_processing=false` means
/// [`crate::SourcePolicy::Retain`]; `true` means
/// [`crate::SourcePolicy::ReleaseAfterProcessing`]. New code should build an
/// [`crate::IngestOptions`] and convert with the `ingest_options()` helpers.
impl LibraryFileIngestOptions {
    pub fn ingest_options(&self) -> crate::IngestOptions {
        crate::IngestOptions::from_legacy(
            Some(self.metadata.clone()),
            self.translation.clone(),
            self.extraction.clone(),
            self.delete_source_after_processing,
        )
    }
}

impl PrepareLibraryUploadRequest {
    /// Canonical [`crate::IngestOptions`] view. Prefers `options` when
    /// present (v0.16 wire); falls back to the flattened v0.15 fields for
    /// in-flight tasks and old clients.
    pub fn ingest_options(&self) -> crate::IngestOptions {
        if let Some(options) = self.options.clone() {
            return options;
        }
        crate::IngestOptions::from_legacy(
            self.metadata.clone(),
            self.translation.clone(),
            self.extraction.clone(),
            self.delete_source_after_processing,
        )
    }

    /// Build a request that carries both shapes: canonical `options` for new
    /// readers plus flattened duplicates for v0.15 readers.
    pub fn with_ingest_options(mut self, options: crate::IngestOptions) -> Self {
        self.delete_source_after_processing = options.as_delete_flag();
        self.translation = options.translation.clone();
        self.extraction = options.extraction.clone();
        self.metadata = options.legacy_metadata_opt();
        self.options = Some(options);
        self
    }
}

impl ImportLibraryFileFromUrlRequest {
    /// Canonical [`crate::IngestOptions`] view. Prefers `options` when
    /// present (v0.16 wire); falls back to the flattened v0.15 fields for
    /// in-flight tasks and old clients.
    pub fn ingest_options(&self) -> crate::IngestOptions {
        if let Some(options) = self.options.clone() {
            return options;
        }
        crate::IngestOptions::from_legacy(
            self.metadata.clone(),
            self.translation.clone(),
            self.extraction.clone(),
            self.delete_source_after_processing,
        )
    }

    /// Build a request that carries both shapes: canonical `options` for new
    /// readers plus flattened duplicates for v0.15 readers.
    pub fn with_ingest_options(mut self, options: crate::IngestOptions) -> Self {
        self.delete_source_after_processing = options.as_delete_flag();
        self.translation = options.translation.clone();
        self.extraction = options.extraction.clone();
        self.metadata = options.legacy_metadata_opt();
        self.options = Some(options);
        self
    }
}
