use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::{
    GroupResponse, ImportLibraryFileFromUrlRequest, LibraryFileUploadMetadata,
    UpsertLibraryTextRequest,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    SourceSync,
    TextBatch,
    FileBatch,
    UrlBatch,
    DeleteBatch,
    Translation,
    VectorRebuild,
}

impl TaskKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SourceSync => "source_sync",
            Self::TextBatch => "text_batch",
            Self::FileBatch => "file_batch",
            Self::UrlBatch => "url_batch",
            Self::DeleteBatch => "delete_batch",
            Self::Translation => "translation",
            Self::VectorRebuild => "vector_rebuild",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Queued,
    Running,
    Waiting,
    Succeeded,
    Failed,
    Cancelled,
}

impl TaskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Waiting => "waiting",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskItemStatus {
    Queued,
    Running,
    Waiting,
    Succeeded,
    Failed,
    Cancelled,
}

impl TaskItemStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Waiting => "waiting",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct TaskRef {
    pub task_id: Uuid,
    #[serde(default)]
    pub item_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct TaskProgress {
    pub total: i64,
    pub queued: i64,
    pub running: i64,
    pub waiting: i64,
    pub succeeded: i64,
    pub failed: i64,
    pub cancelled: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskOrigin {
    Manual,
    Rerun,
}

impl TaskOrigin {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Rerun => "rerun",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct TaskResponse {
    pub task_id: Uuid,
    pub kind: TaskKind,
    pub status: TaskStatus,
    pub origin: TaskOrigin,
    pub group_path: Option<String>,
    pub source_key: Option<String>,
    pub stage: Option<String>,
    pub waiting_reason: Option<String>,
    pub dependency_key: Option<String>,
    pub progress: TaskProgress,
    pub failure_stage: Option<String>,
    pub error_summary: Option<String>,
    pub eta_seconds: Option<i64>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct TaskItemResponse {
    pub item_id: Uuid,
    pub ordinal: i32,
    pub status: TaskItemStatus,
    pub resource_id: Option<String>,
    pub file_id: Option<Uuid>,
    pub stage: Option<String>,
    pub waiting_reason: Option<String>,
    pub dependency_key: Option<String>,
    pub next_attempt_at: Option<DateTime<Utc>>,
    pub failure_stage: Option<String>,
    pub error_message: Option<String>,
    pub attempt_count: i32,
    pub retryable: bool,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, IntoParams, ToSchema)]
#[into_params(parameter_in = Query)]
pub struct TaskListQuery {
    #[serde(default = "default_page")]
    pub page: u32,
    #[serde(default = "default_page_size")]
    pub page_size: u32,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub kind: Option<TaskKind>,
    #[serde(default)]
    pub status: Option<TaskStatus>,
    #[serde(default)]
    pub stage: Option<String>,
    #[serde(default)]
    pub waiting_reason: Option<String>,
    #[serde(default)]
    pub dependency_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct TaskPageResponse {
    pub items: Vec<TaskResponse>,
    pub pagination: crate::Pagination,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct TaskItemsResponse {
    pub items: Vec<TaskItemResponse>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, IntoParams, ToSchema)]
#[into_params(parameter_in = Query)]
pub struct TaskItemsQuery {
    #[serde(default = "default_item_limit")]
    pub limit: u32,
    #[serde(default)]
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct ScopeSpec {
    pub group_path: String,
    pub name: String,
    pub visibility: crate::Visibility,
    #[serde(default)]
    pub kind: Option<crate::GroupKind>,
    #[serde(default)]
    pub metadata_indexes: Vec<ScopeMetadataIndex>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct ScopeMetadataIndex {
    pub source_key: String,
    #[serde(flatten)]
    pub definition: crate::CreateMetadataIndexRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EnsureScopeResponse {
    pub group: GroupResponse,
    pub metadata_indexes: Vec<crate::MetadataIndexResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TextBatchRequest {
    pub items: Vec<UpsertLibraryTextRequest>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UrlBatchRequest {
    pub items: Vec<ImportLibraryFileFromUrlRequest>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DeleteBatchRequest {
    pub items: Vec<crate::DocumentKey>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct FileBatchItem {
    pub filename: String,
    pub media_type: String,
    pub content_base64: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declared_sha256: Option<String>,
    #[serde(default)]
    pub folder_id: Option<Uuid>,
    #[serde(default)]
    pub metadata: Option<LibraryFileUploadMetadata>,
    #[serde(default)]
    pub translation: Option<crate::TranslationDirective>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct FileBatchRequest {
    pub items: Vec<FileBatchItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TaskSubmitRequest {
    /// Re-process existing library files that already have a file id.
    RetryFileBatch {
        #[serde(default)]
        group_path: Option<String>,
        items: Vec<FileRetryItem>,
    },
    /// Ingest file contents uploaded inline as base64.
    FileBatch {
        #[serde(default)]
        group_path: Option<String>,
        items: Vec<FileBatchItem>,
    },
    TextBatch {
        #[serde(default)]
        group_path: Option<String>,
        items: Vec<UpsertLibraryTextRequest>,
    },
    UrlBatch {
        #[serde(default)]
        group_path: Option<String>,
        items: Vec<ImportLibraryFileFromUrlRequest>,
    },
    DeleteBatch {
        #[serde(default)]
        group_path: Option<String>,
        items: Vec<crate::DocumentKey>,
    },
    SourceSync {
        #[serde(default)]
        group_path: Option<String>,
        source_key: String,
    },
    TranslationBatch {
        #[serde(default)]
        group_path: Option<String>,
        items: Vec<TranslationSubmitItem>,
    },
    VectorRebuild,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct FileRetryItem {
    pub file_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct TranslationSubmitItem {
    pub document_id: i64,
    #[serde(default)]
    pub target_locales: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct TaskRetryResponse {
    pub task: TaskRef,
    pub retried_items: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct RerunTaskResponse {
    pub task: TaskRef,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskPurgeMode {
    Expired,
    AllTerminal,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct TaskMaintenanceSettings {
    pub cleanup_enabled: bool,
    pub retention_days: i64,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct TaskMaintenanceStats {
    pub total: i64,
    pub queued: i64,
    pub running: i64,
    pub waiting: i64,
    pub succeeded: i64,
    pub failed: i64,
    pub cancelled: i64,
    pub active: i64,
    pub expired_terminal: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct TaskMaintenanceOverview {
    pub settings: TaskMaintenanceSettings,
    pub stats: TaskMaintenanceStats,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct UpdateTaskMaintenanceSettingsRequest {
    pub cleanup_enabled: bool,
    pub retention_days: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct CancelActiveTasksResponse {
    pub cancelled_tasks: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct PurgeTasksRequest {
    pub mode: TaskPurgeMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct PurgeTasksResponse {
    pub deleted_tasks: i64,
}

fn default_page() -> u32 {
    1
}

fn default_page_size() -> u32 {
    50
}

fn default_item_limit() -> u32 {
    100
}
