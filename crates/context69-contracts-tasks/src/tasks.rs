use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use context69_contracts_core::Visibility;
use context69_contracts_core::common::Pagination;
use context69_contracts_core::pagination::SortDirection;

use context69_contracts_library::{
    ImportLibraryFileFromUrlRequest, LibraryFileUploadMetadata, UpsertLibraryTextRequest,
};
use context69_contracts_namespace::GroupResponse;

pub use context69_contracts_core::TaskRef;

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
    /// Non-null while the task history row is in the recycle bin. Trashing
    /// only soft-deletes the task record; files, processed text, and vectors
    /// are never affected.
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct ExternalJobInfo {
    pub provider: String,
    pub remote_task_id: String,
    pub status: String,
    pub remote_status: Option<String>,
    pub submitted_at: DateTime<Utc>,
    pub last_polled_at: Option<DateTime<Utc>>,
    pub next_poll_at: Option<DateTime<Utc>>,
    pub deadline_at: Option<DateTime<Utc>>,
    pub error_message: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_job: Option<ExternalJobInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskSortBy {
    CreatedAt,
    UpdatedAt,
    Status,
    Kind,
    Stage,
    GroupPath,
}

impl TaskSortBy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CreatedAt => "created_at",
            Self::UpdatedAt => "updated_at",
            Self::Status => "status",
            Self::Kind => "kind",
            Self::Stage => "stage",
            Self::GroupPath => "group_path",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskListView {
    Processing,
    Completed,
    Trash,
}

impl TaskListView {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Processing => "processing",
            Self::Completed => "completed",
            Self::Trash => "trash",
        }
    }
}

/// v0.15 task list query kept for wire compatibility.
///
/// Deprecated `trashed` stays so legacy callers keep compiling; new code
/// should use [`CanonicalTaskListQuery`], which requires a typed `view` and
/// carries no `trashed` flag.
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
    /// When true, list only trashed tasks; when false or omitted, list only
    /// active (non-trashed) tasks. Trashed rows stay reachable by id for their
    /// owner (for example to restore them) but never appear in active lists.
    /// Ignored when `view` is set: the view owns the trash predicate.
    #[serde(default)]
    pub trashed: Option<bool>,
    /// Typed list view. `processing` lists non-trashed tasks whose status is
    /// not `succeeded`; `completed` lists non-trashed `succeeded` tasks;
    /// `trash` lists trashed tasks. A user-supplied `status` further narrows
    /// the view and never widens it (for example `processing` plus
    /// `status=succeeded` matches nothing). When omitted, the legacy
    /// `trashed`/`status` filters apply for external callers.
    #[serde(default)]
    pub view: Option<TaskListView>,
    #[serde(default)]
    pub stage: Option<String>,
    #[serde(default)]
    pub waiting_reason: Option<String>,
    #[serde(default)]
    pub dependency_key: Option<String>,
    #[serde(default)]
    pub sort_by: Option<TaskSortBy>,
    #[serde(default)]
    pub sort_direction: Option<SortDirection>,
}

/// v0.16 canonical task list query: typed `view` is required and the legacy
/// `trashed` flag is gone. Offset bounds match [`context69_contracts_core::pagination::OffsetPageQuery`].
#[derive(Debug, Clone, Serialize, Deserialize, IntoParams, ToSchema, JsonSchema)]
#[into_params(parameter_in = Query)]
pub struct CanonicalTaskListQuery {
    #[serde(default = "default_page")]
    #[param(minimum = 1, maximum = 10_000)]
    #[schema(minimum = 1, maximum = 10_000)]
    #[schemars(range(min = 1, max = 10_000))]
    pub page: u32,
    #[serde(default = "default_page_size")]
    #[param(minimum = 1, maximum = 100)]
    #[schema(minimum = 1, maximum = 100)]
    #[schemars(range(min = 1, max = 100))]
    pub page_size: u32,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub kind: Option<TaskKind>,
    #[serde(default)]
    pub status: Option<TaskStatus>,
    pub view: TaskListView,
    #[serde(default)]
    pub stage: Option<String>,
    #[serde(default)]
    pub waiting_reason: Option<String>,
    #[serde(default)]
    pub dependency_key: Option<String>,
    #[serde(default)]
    pub sort_by: Option<TaskSortBy>,
    #[serde(default)]
    pub sort_direction: Option<SortDirection>,
}

impl CanonicalTaskListQuery {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.page == 0 || self.page > 10_000 {
            return Err(anyhow::anyhow!("page must be between 1 and 10000"));
        }
        if self.page_size == 0 || self.page_size > 100 {
            return Err(anyhow::anyhow!("page_size must be between 1 and 100"));
        }
        Ok(())
    }
}

impl From<CanonicalTaskListQuery> for TaskListQuery {
    fn from(canonical: CanonicalTaskListQuery) -> Self {
        Self {
            page: canonical.page,
            page_size: canonical.page_size,
            query: canonical.query,
            kind: canonical.kind,
            status: canonical.status,
            trashed: None,
            view: Some(canonical.view),
            stage: canonical.stage,
            waiting_reason: canonical.waiting_reason,
            dependency_key: canonical.dependency_key,
            sort_by: canonical.sort_by,
            sort_direction: canonical.sort_direction,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct TaskPageResponse {
    pub items: Vec<TaskResponse>,
    pub pagination: Pagination,
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
    pub visibility: Visibility,
    #[serde(default)]
    pub kind: Option<context69_contracts_namespace::GroupKind>,
    #[serde(default)]
    pub metadata_indexes: Vec<ScopeMetadataIndex>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct ScopeMetadataIndex {
    pub source_key: String,
    #[serde(flatten)]
    pub definition: context69_contracts_search::CreateMetadataIndexRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EnsureScopeResponse {
    pub group: GroupResponse,
    pub metadata_indexes: Vec<context69_contracts_search::MetadataIndexResponse>,
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
    pub items: Vec<context69_contracts_search::DocumentKey>,
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
    /// Canonical ingest options. When present, takes precedence over the
    /// deprecated flattened fields below. New code should send only this;
    /// v0.15 payloads send only the flattened fields and stay readable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<context69_contracts_library::IngestOptions>,
    #[serde(default)]
    pub metadata: Option<LibraryFileUploadMetadata>,
    #[serde(default)]
    pub translation: Option<context69_contracts_translation::TranslationDirective>,
    #[serde(default)]
    pub extraction: Option<context69_contracts_extraction::ExtractionDirective>,
    /// Release the source object once processing succeeds. Chosen once at
    /// upload; defaults to `false` (retain the source).
    /// Deprecated: use `options.source_policy` instead.
    #[serde(default)]
    pub delete_source_after_processing: bool,
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
        items: Vec<context69_contracts_search::DocumentKey>,
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
    /// Uncertain `submitting` Docling rows (all ages, terminal parents or not).
    /// These rows never count as remotely cancelled and need explicit review.
    #[serde(default)]
    pub uncertain_submitting: i64,
    /// Stale placeholder `submitting` rows on terminal parents that the admin
    /// quarantine API is allowed to isolate as `orphaned`.
    #[serde(default)]
    pub quarantinable_submitting: i64,
    /// Rows already isolated as `orphaned`: non-active, no longer blocking
    /// terminal-task cleanup/purge or Docling admission.
    #[serde(default)]
    pub orphaned_external_jobs: i64,
    /// Persisted Docling remote-slot ceiling (`docling_settings.max_inflight`,
    /// default 1 when unconfigured). Read-only capacity signal; the overview
    /// never tunes admission.
    #[serde(default)]
    pub docling_max_inflight: i64,
    /// Due admission-deferred `waiting/backoff` items carrying the
    /// `remote admission is full` marker whose deferral delay has elapsed.
    /// Read-only backpressure signal paired with `docling_max_inflight`.
    #[serde(default)]
    pub due_docling_waiting_count: i64,
    /// Oldest `submitted_at` among uncertain `submitting` Docling rows.
    /// `None` when no such row exists. Read-only age signal.
    #[serde(default)]
    pub oldest_uncertain_submitting_at: Option<DateTime<Utc>>,
    /// Oldest `submitted_at` among quarantinable `submitting` rows (same
    /// eligibility as `quarantinable_submitting`). `None` when empty.
    /// Read-only age signal.
    #[serde(default)]
    pub oldest_quarantinable_submitting_at: Option<DateTime<Utc>>,
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

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct RecoverDoclingTaskRequest {
    /// Free-form human justification recorded in the recovery audit.
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct RecoveredDoclingTask {
    pub task_id: Uuid,
    pub item_id: Uuid,
    pub old_remote_task_id: Option<String>,
    pub old_remote_status: Option<String>,
    pub new_remote_task_id: String,
    pub new_stage: String,
    pub file_id: Option<Uuid>,
    pub recovered_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct RecoverDoclingTaskResponse {
    pub recovered: RecoveredDoclingTask,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct QueueDoclingRecoveryRequest {
    /// Free-form human justification recorded in operator logs.
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct QueuedDoclingTask {
    pub task_id: Uuid,
    pub item_id: Uuid,
    /// Stage the item was parked on for dispatcher pickup (`docling`).
    pub stage: String,
    pub file_id: Option<Uuid>,
    pub queued_at: DateTime<Utc>,
    /// True when the item was already queued and no state changed: no new
    /// attempt row and no new remote job were created.
    #[serde(default)]
    pub already_queued: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct QueueDoclingRecoveryResponse {
    pub queued: QueuedDoclingTask,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct QuarantineStaleSubmittingRequest {
    /// Free-form human justification stored on each row and its audit row.
    pub reason: String,
    /// Only rows older than this many minutes are eligible. Defaults to 30,
    /// must be between 10 and 10080 (one week).
    #[serde(default)]
    pub grace_minutes: Option<i64>,
    /// Maximum rows to quarantine per call. Defaults to 100, clamped to
    /// 1..=1000.
    #[serde(default)]
    pub limit: Option<i64>,
    /// When true, only read eligibility counts and return a preview without
    /// mutating any row or writing any audit row. Optional for backward
    /// compatibility; omitted or false preserves the mutating behavior.
    #[serde(default)]
    pub dry_run: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct QuarantinedExternalJob {
    pub external_job_id: Uuid,
    pub task_id: Uuid,
    pub item_id: Uuid,
    pub old_remote_task_id: Option<String>,
    pub quarantined_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
pub struct QuarantineStaleSubmittingResponse {
    pub quarantined: Vec<QuarantinedExternalJob>,
    pub quarantined_count: i64,
    /// Still `submitting` because the parent task/item is not terminal.
    pub skipped_non_terminal: i64,
    /// Still `submitting` because it is newer than the grace cutoff.
    pub skipped_fresh: i64,
    /// Still `submitting` with a non-placeholder remote id; needs manual
    /// review because a real remote job may exist.
    pub skipped_real_remote: i64,
    /// True when this response is a dry-run preview: no row was mutated and
    /// no audit row was written. Always present; false for mutating calls so
    /// callers never conflate eligible rows with changed rows.
    #[serde(default)]
    pub dry_run: bool,
    /// Total eligible (`quarantinable`) `submitting` rows at read time:
    /// placeholder remote id, older than the grace cutoff, terminal parents.
    /// Uncapped by `limit`. In dry-run mode this is the full preview total
    /// with `quarantined` empty and `quarantined_count` zero. In mutating
    /// mode this is the remainder still eligible after this call, while
    /// `quarantined_count` is the actual number of rows changed.
    #[serde(default)]
    pub quarantinable_count: i64,
}

fn default_page() -> u32 {
    context69_contracts_core::pagination::default_page()
}

fn default_page_size() -> u32 {
    context69_contracts_core::pagination::default_page_size()
}

fn default_item_limit() -> u32 {
    100
}

impl FileBatchItem {
    /// Canonical [`context69_contracts_library::IngestOptions`] view. Prefers `options` when
    /// present (v0.16 wire); falls back to the flattened v0.15 fields for
    /// in-flight tasks and old clients.
    pub fn ingest_options(&self) -> context69_contracts_library::IngestOptions {
        if let Some(options) = self.options.clone() {
            return options;
        }
        context69_contracts_library::IngestOptions::from_legacy(
            self.metadata.clone(),
            self.translation.clone(),
            self.extraction.clone(),
            self.delete_source_after_processing,
        )
    }

    /// Build an item that carries both shapes: canonical `options` for new
    /// readers plus flattened duplicates for v0.15 readers.
    pub fn with_ingest_options(
        mut self,
        options: context69_contracts_library::IngestOptions,
    ) -> Self {
        self.delete_source_after_processing = options.as_delete_flag();
        self.translation = options.translation.clone();
        self.extraction = options.extraction.clone();
        self.metadata = options.legacy_metadata_opt();
        self.options = Some(options);
        self
    }
}
