use std::time::{Duration, Instant};

pub use context69_contracts::{
    ApiErrorCode, AuthMeResponse, AuthUserResponse, BatchGetDocumentsRequest,
    BatchGetDocumentsResponse, CancelActiveTasksResponse, CanonicalApiErrorResponse,
    CanonicalSearchRequest, CanonicalTaskListQuery, CanonicalUpdateSearchSettingsRequest,
    CanonicalUploadMetadata, CreateDocumentResultRequest, CreateMetadataIndexRequest,
    CursorPageQuery, CursorPagination, DeleteBatchRequest, DoclingRemoteTasksQuery,
    DoclingRemoteTasksResponse, DocumentChunkResponse, DocumentKey, DocumentResponse,
    DocumentResultProvenance, DocumentResultResponse, EnsureScopeResponse, FileBatchItem,
    FileBatchRequest, GroupKind, GroupResponse, HealthResponse,
    ImportLibraryFileFromUrlRequest as UrlBatchItem, IngestOptions, LibraryFileDetailResponse,
    LibraryTextContentFormat as TextContentFormat, ListDocumentResultsQuery,
    ListDocumentResultsResponse, MetadataDataType, MetadataFilter, MetadataFilterOperator,
    MetadataValueKind, OffsetPageQuery, OffsetPagination, RebuildControlRequest, RerunTaskResponse,
    ScopeMetadataIndex, ScopeSpec, SearchMode, SearchRequest, SearchResponse, SearchSort,
    SecretPatch, SortDirection, SourcePolicy, TaskBatchLifecycleOutcome, TaskBatchLifecycleReason,
    TaskBatchLifecycleRequest, TaskBatchLifecycleResponse, TaskBatchRerunRequest,
    TaskBatchRerunResponse, TaskBatchRetryRequest, TaskBatchRetryResponse, TaskBatchTrashRequest,
    TaskDiagnoseResponse, TaskItemResponse, TaskItemStatus, TaskItemsQuery, TaskItemsResponse,
    TaskKind, TaskListQuery, TaskListView, TaskPageResponse, TaskProgress, TaskQueueStateResponse,
    TaskRef, TaskResponse, TaskRetryResponse, TaskSortBy, TaskStatus, TaskSubmitRequest,
    TextBatchRequest, TranslationDirective, TranslationStatus, UpdateTaskQueueStateRequest,
    UpsertLibraryTextRequest as TextBatchItem, UrlBatchRequest, VectorIndexRebuildStatus,
    Visibility,
};
use reqwest::{Method, StatusCode};
use uuid::Uuid;

pub use context69_contracts::search::SearchPagination;

/// Build the HTTP-status error the legacy single-task call returned for one
/// refusal, including the canonical `ApiErrorResponse` JSON body. The batch
/// endpoint has no response body per item, so the status class and message are
/// reconstructed from the legacy contract.
fn canonical_refusal(status: StatusCode, message: &str) -> Error {
    let code = ApiErrorCode::code_for_status(status.as_u16());
    let body = serde_json::to_string(&CanonicalApiErrorResponse::new(code, message.to_string()))
        .unwrap_or_default();
    Error::HttpStatus {
        status,
        api_error: Some(message.to_string()),
        body,
    }
}

/// Classify a masked lifecycle refusal after re-reading the task.
///
/// A readable task that the batch masked as unavailable is readable but not
/// manageable, which the legacy route reported as `403`. A read that is itself
/// `404` keeps the legacy `404`. Any other read failure propagates unchanged;
/// the read never falls back to a mutating call.
fn classify_masked_read<T>(result: Result<T, Error>) -> Error {
    match result {
        Ok(_) => canonical_refusal(StatusCode::FORBIDDEN, "task management permission denied"),
        Err(Error::HttpStatus { status, .. }) if status == StatusCode::NOT_FOUND => {
            canonical_refusal(StatusCode::NOT_FOUND, "task not found")
        }
        Err(error) => error,
    }
}

use super::{Context69Client, transport::group_path};
use crate::Error;

#[derive(Debug, Clone)]
pub struct WaitOptions {
    pub timeout: Duration,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
}

impl Default for WaitOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30 * 60),
            initial_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_secs(2),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CompactSearchHit {
    pub document_id: i64,
    pub external_id: String,
    pub title: String,
    pub summary: Option<String>,
    pub source_uri: String,
    pub published_at: Option<chrono::DateTime<chrono::Utc>>,
    pub score: f32,
    pub snippet: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CompactSearchResponse {
    pub query: String,
    pub hits: Vec<CompactSearchHit>,
    pub pagination: SearchPagination,
}

impl CompactSearchResponse {
    pub fn next_cursor(&self) -> Option<&str> {
        self.pagination.next_cursor.as_deref()
    }

    pub fn has_more(&self) -> bool {
        self.pagination.has_more.unwrap_or(false)
    }
}

/// v0.16 canonical task list options. Maps 1:1 onto
/// [`CanonicalTaskListQuery`]: `view` is required, `page` is `1..=10_000`
/// and `page_size` is `1..=100`. The default (`processing`, page 1,
/// page size 25) matches the processing-queue first page.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TaskListOptions {
    pub view: TaskListView,
    pub page: u32,
    pub page_size: u32,
    pub query: Option<String>,
    pub kind: Option<TaskKind>,
    pub status: Option<TaskStatus>,
    pub stage: Option<String>,
    pub waiting_reason: Option<String>,
    pub dependency_key: Option<String>,
    pub sort_by: Option<TaskSortBy>,
    pub sort_direction: Option<SortDirection>,
}

impl Default for TaskListOptions {
    fn default() -> Self {
        Self {
            view: TaskListView::Processing,
            page: 1,
            page_size: 25,
            query: None,
            kind: None,
            status: None,
            stage: None,
            waiting_reason: None,
            dependency_key: None,
            sort_by: None,
            sort_direction: None,
        }
    }
}

impl TaskListOptions {
    pub fn validate(&self) -> Result<(), Error> {
        if self.page == 0 || self.page > 10_000 {
            return Err(Error::InvalidResponse(format!(
                "page must be between 1 and 10000, got {}",
                self.page
            )));
        }
        if self.page_size == 0 || self.page_size > 100 {
            return Err(Error::InvalidResponse(format!(
                "page_size must be between 1 and 100, got {}",
                self.page_size
            )));
        }
        Ok(())
    }

    pub fn as_canonical_query(&self) -> Result<CanonicalTaskListQuery, Error> {
        self.validate()?;
        Ok(CanonicalTaskListQuery {
            page: self.page,
            page_size: self.page_size,
            query: self.query.clone(),
            kind: self.kind,
            status: self.status,
            view: self.view,
            stage: self.stage.clone(),
            waiting_reason: self.waiting_reason.clone(),
            dependency_key: self.dependency_key.clone(),
            sort_by: self.sort_by,
            sort_direction: self.sort_direction,
        })
    }
}

/// v0.16 canonical task item window. `limit` is `1..=100` (aligned to the
/// shared cursor kernel and the `TaskItemsQuery` wire default of 100);
/// `cursor` is the opaque offset token from `TaskItemsResponse::next_cursor`
/// and is omitted when `None` (no hidden `cursor=0` default). `status`
/// narrows to one `TaskItemStatus`; `None` lists every status. `cursor` is
/// scoped to `status`: reset to no cursor when `status` changes (issue 413).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TaskItemsOptions {
    pub limit: u32,
    pub cursor: Option<String>,
    #[serde(default)]
    pub status: Option<TaskItemStatus>,
}

impl Default for TaskItemsOptions {
    fn default() -> Self {
        Self {
            limit: 100,
            cursor: None,
            status: None,
        }
    }
}

impl TaskItemsOptions {
    pub fn validate(&self) -> Result<(), Error> {
        if self.limit == 0 || self.limit > 100 {
            return Err(Error::InvalidResponse(format!(
                "limit must be between 1 and 100, got {}",
                self.limit
            )));
        }
        Ok(())
    }

    pub fn as_wire_query(&self) -> Result<TaskItemsQuery, Error> {
        self.validate()?;
        Ok(TaskItemsQuery {
            limit: self.limit,
            cursor: self.cursor.clone(),
            status: self.status,
        })
    }
}

impl Context69Client {
    pub async fn ensure_scope(&self, spec: &ScopeSpec) -> Result<EnsureScopeResponse, Error> {
        self.execute_json(
            self.authorized_request(Method::POST, "/v1/scopes/ensure")
                .await?
                .json(spec),
        )
        .await
    }

    pub async fn submit_text_batch(
        &self,
        group_path_value: &str,
        request: &TextBatchRequest,
    ) -> Result<TaskRef, Error> {
        self.submit_batch(group_path(group_path_value, "/batch/text"), request)
            .await
    }

    pub async fn submit_url_batch(
        &self,
        group_path_value: &str,
        request: &UrlBatchRequest,
    ) -> Result<TaskRef, Error> {
        self.submit_batch(group_path(group_path_value, "/batch/url"), request)
            .await
    }

    pub async fn submit_file_batch(
        &self,
        group_path_value: &str,
        request: &FileBatchRequest,
    ) -> Result<TaskRef, Error> {
        self.submit_batch(group_path(group_path_value, "/batch/file"), request)
            .await
    }

    pub async fn submit_delete_batch(
        &self,
        group_path_value: &str,
        request: &DeleteBatchRequest,
    ) -> Result<TaskRef, Error> {
        self.submit_batch(group_path(group_path_value, "/batch/delete"), request)
            .await
    }

    /// Release a succeeded file's source object. The file record, its
    /// processed text, and its vectors are retained.
    pub async fn release_file_source(
        &self,
        group_path_value: &str,
        file_id: Uuid,
    ) -> Result<LibraryFileDetailResponse, Error> {
        let path = group_path(
            group_path_value,
            &format!("/library/files/{file_id}/release-source"),
        );
        self.execute_json(self.authorized_request(Method::POST, &path).await?)
            .await
    }

    pub async fn submit_task(&self, request: &TaskSubmitRequest) -> Result<TaskRef, Error> {
        self.submit_batch("/v1/tasks".to_string(), request).await
    }

    async fn submit_batch<T: serde::Serialize>(
        &self,
        path: String,
        body: &T,
    ) -> Result<TaskRef, Error> {
        let key = idempotency_key(&path, body)?;
        self.execute_json(
            self.authorized_request(Method::POST, &path)
                .await?
                .header("Idempotency-Key", key)
                .json(body),
        )
        .await
    }

    /// Canonical `get_task` (`GET /v1/tasks/{task_id}`).
    pub async fn get_task(&self, task_id: Uuid) -> Result<TaskResponse, Error> {
        let path = format!("/v1/tasks/{task_id}");
        self.execute_json(self.authorized_request(Method::GET, &path).await?)
            .await
    }

    /// Canonical `list_tasks` (`GET /v1/tasks` with [`CanonicalTaskListQuery`]).
    /// The typed `view` is always sent; page/page_size are validated
    /// (`1..=10_000` / `1..=100`) before the request leaves the SDK.
    pub async fn list_tasks(&self, options: &TaskListOptions) -> Result<TaskPageResponse, Error> {
        let query = options.as_canonical_query()?;
        self.execute_json(
            self.authorized_request(Method::GET, "/v1/tasks")
                .await?
                .query(&query),
        )
        .await
    }

    /// Canonical `list_task_items` (`GET /v1/tasks/{task_id}/items`).
    /// `limit` is validated (`1..=100`); `cursor` is omitted when `None`
    /// so the server applies its own default instead of a hidden `0`.
    /// `status` is omitted when `None` (lists every status); `cursor` is
    /// scoped to `status` (issue 413).
    pub async fn list_task_items(
        &self,
        task_id: Uuid,
        options: &TaskItemsOptions,
    ) -> Result<TaskItemsResponse, Error> {
        let query = options.as_wire_query()?;
        let path = format!("/v1/tasks/{task_id}/items");
        let mut pairs = vec![("limit".to_string(), query.limit.to_string())];
        if let Some(cursor) = query.cursor {
            pairs.push(("cursor".to_string(), cursor));
        }
        if let Some(status) = query.status {
            pairs.push(("status".to_string(), status.as_str().to_string()));
        }
        self.execute_json(
            self.authorized_request(Method::GET, &path)
                .await?
                .query(&pairs),
        )
        .await
    }

    /// Canonical `diagnose_task` (`GET /v1/tasks/{task_id}/diagnose`).
    ///
    /// Read-only operator detail: the parent projection, its items in ordinal
    /// order with active/latest attempt forensics, the dependency gates those
    /// items can wait on, and the parent/item consistency verdict. Scoped by
    /// the same task-detail authorization as [`Self::get_task`], so a foreign
    /// task reads as `404`.
    pub async fn diagnose_task(&self, task_id: Uuid) -> Result<TaskDiagnoseResponse, Error> {
        let path = format!("/v1/tasks/{task_id}/diagnose");
        self.execute_json(self.authorized_request(Method::GET, &path).await?)
            .await
    }

    pub async fn wait(&self, task_id: Uuid, timeout: Duration) -> Result<TaskResponse, Error> {
        self.wait_with_options(
            task_id,
            WaitOptions {
                timeout,
                ..WaitOptions::default()
            },
        )
        .await
    }

    pub async fn wait_with_options(
        &self,
        task_id: Uuid,
        options: WaitOptions,
    ) -> Result<TaskResponse, Error> {
        if options.timeout.is_zero()
            || options.initial_backoff.is_zero()
            || options.max_backoff.is_zero()
        {
            return Err(Error::InvalidTimeout(options.timeout));
        }
        let started = Instant::now();
        let mut delay = options.initial_backoff;
        loop {
            if started.elapsed() >= options.timeout {
                return Err(Error::TaskWaitTimeout {
                    task_id,
                    timeout: options.timeout,
                });
            }
            match self.get_task(task_id).await {
                Ok(task) => {
                    if matches!(
                        task.status,
                        context69_contracts::TaskStatus::Succeeded
                            | context69_contracts::TaskStatus::Failed
                            | context69_contracts::TaskStatus::Cancelled
                    ) {
                        return Ok(task);
                    }
                }
                Err(error) if error.is_retryable() => {}
                Err(error) => return Err(error),
            }
            tokio::time::sleep(delay.min(options.timeout.saturating_sub(started.elapsed()))).await;
            delay = (delay + delay).min(options.max_backoff);
        }
    }

    pub async fn retry_task(&self, task_id: Uuid) -> Result<TaskRetryResponse, Error> {
        let path = format!("/v1/tasks/{task_id}/retry");
        self.execute_json(self.authorized_request(Method::POST, &path).await?)
            .await
    }

    /// Recover up to 100 failed/cancelled tasks in one bounded call. Each id
    /// keeps its requested position in the ordered results.
    pub async fn batch_retry_tasks(
        &self,
        request: &TaskBatchRetryRequest,
    ) -> Result<TaskBatchRetryResponse, Error> {
        let path = "/v1/tasks/batch/retry";
        self.execute_json(
            self.authorized_request(Method::POST, path)
                .await?
                .json(request),
        )
        .await
    }

    /// Rerun up to 100 succeeded tasks from their current source in one
    /// bounded call. Each id keeps its requested position in the ordered
    /// results, and only an accepted id carries the created child task id.
    pub async fn batch_rerun_tasks(
        &self,
        request: &TaskBatchRerunRequest,
    ) -> Result<TaskBatchRerunResponse, Error> {
        let path = "/v1/tasks/batch/rerun";
        self.execute_json(
            self.authorized_request(Method::POST, path)
                .await?
                .json(request),
        )
        .await
    }

    /// Cancel up to 100 tasks in one bounded call. Active ids are cancelled and
    /// every other id keeps its requested position with a per-id reason.
    pub async fn batch_cancel_tasks(
        &self,
        request: &TaskBatchLifecycleRequest,
    ) -> Result<TaskBatchLifecycleResponse, Error> {
        self.post_batch_lifecycle("/v1/tasks/batch/cancel", request)
            .await
    }

    /// Move up to 100 tasks to the recycle bin in one bounded call. The
    /// request opts into cancelling active rows first; the default keeps the
    /// terminal-only contract.
    pub async fn batch_trash_tasks(
        &self,
        request: &TaskBatchTrashRequest,
    ) -> Result<TaskBatchLifecycleResponse, Error> {
        self.post_batch_lifecycle("/v1/tasks/batch/trash", request)
            .await
    }

    /// Restore up to 100 trashed tasks in one bounded call.
    pub async fn batch_restore_tasks(
        &self,
        request: &TaskBatchLifecycleRequest,
    ) -> Result<TaskBatchLifecycleResponse, Error> {
        self.post_batch_lifecycle("/v1/tasks/batch/restore", request)
            .await
    }

    /// Permanently delete up to 100 trashed tasks in one bounded call.
    pub async fn batch_purge_tasks(
        &self,
        request: &TaskBatchLifecycleRequest,
    ) -> Result<TaskBatchLifecycleResponse, Error> {
        self.post_batch_lifecycle("/v1/tasks/batch/purge", request)
            .await
    }

    async fn post_batch_lifecycle<T: serde::Serialize>(
        &self,
        path: &str,
        request: &T,
    ) -> Result<TaskBatchLifecycleResponse, Error> {
        self.execute_json(
            self.authorized_request(Method::POST, path)
                .await?
                .json(request),
        )
        .await
    }

    /// Submit one id to a batch lifecycle endpoint and turn a per-item refusal
    /// into the HTTP-status error the single-task contract reports.
    ///
    /// `ineligible_message` is the legacy conflict message for this operation.
    async fn batch_lifecycle_one<T: serde::Serialize>(
        &self,
        path: &str,
        request: &T,
        task_id: Uuid,
        ineligible_message: &str,
    ) -> Result<(), Error> {
        let response = self.post_batch_lifecycle(path, request).await?;
        match response.results.iter().find(|item| item.task_id == task_id) {
            Some(item) if item.outcome == TaskBatchLifecycleOutcome::Accepted => Ok(()),
            Some(item) => Err(self
                .lifecycle_refusal(task_id, item.reason, ineligible_message)
                .await),
            None => Err(Error::InvalidResponse(format!(
                "the batch response omitted task {task_id}"
            ))),
        }
    }

    /// Map one batch refusal reason onto the legacy single-task error.
    ///
    /// Batch existence masking reports an unmanageable readable task as
    /// `not_found`; a read-only `get_task` distinguishes that `403` from a
    /// genuinely missing `404`. The read is never a fallback mutating call.
    async fn lifecycle_refusal(
        &self,
        task_id: Uuid,
        reason: Option<TaskBatchLifecycleReason>,
        ineligible_message: &str,
    ) -> Error {
        match reason {
            Some(TaskBatchLifecycleReason::NotFound) => {
                classify_masked_read(self.get_task(task_id).await)
            }
            Some(TaskBatchLifecycleReason::IneligibleStatus) => {
                canonical_refusal(StatusCode::CONFLICT, ineligible_message)
            }
            Some(TaskBatchLifecycleReason::InternalError) | None => canonical_refusal(
                StatusCode::INTERNAL_SERVER_ERROR,
                "task lifecycle action failed",
            ),
        }
    }

    pub async fn rerun_task(&self, task_id: Uuid) -> Result<RerunTaskResponse, Error> {
        let path = format!("/v1/tasks/{task_id}/rerun");
        self.execute_json(self.authorized_request(Method::POST, &path).await?)
            .await
    }

    /// Cancel one task through the bounded batch endpoint. A missing task is
    /// reported as `404`, a readable-but-unmanageable task as `403`, and a
    /// terminal task as `409`, as the single route did.
    pub async fn cancel_task(&self, task_id: Uuid) -> Result<(), Error> {
        let request = TaskBatchLifecycleRequest {
            task_ids: vec![task_id],
        };
        self.batch_lifecycle_one(
            "/v1/tasks/batch/cancel",
            &request,
            task_id,
            "task is already terminal or not found",
        )
        .await
    }

    /// Move a terminal task's history into the recycle bin through the bounded
    /// batch endpoint. Terminal-only: an active task is reported as `409` and
    /// its work keeps running, exactly like the single route. Files and
    /// processed results are never affected.
    pub async fn trash_task(&self, task_id: Uuid) -> Result<TaskResponse, Error> {
        let request = TaskBatchTrashRequest {
            task_ids: vec![task_id],
            cancel_active: false,
        };
        self.batch_lifecycle_one(
            "/v1/tasks/batch/trash",
            &request,
            task_id,
            "active task cannot be trashed until it reaches a terminal state",
        )
        .await?;
        self.get_task(task_id).await
    }

    /// Restore a trashed task's history through the bounded batch endpoint.
    /// Idempotent.
    pub async fn restore_task(&self, task_id: Uuid) -> Result<TaskResponse, Error> {
        let request = TaskBatchLifecycleRequest {
            task_ids: vec![task_id],
        };
        self.batch_lifecycle_one(
            "/v1/tasks/batch/restore",
            &request,
            task_id,
            "task cannot be restored",
        )
        .await?;
        self.get_task(task_id).await
    }

    /// Permanently delete a trashed task's history through the bounded batch
    /// endpoint. Only trashed rows qualify; a non-trashed task is reported as
    /// `409`.
    pub async fn purge_trashed_task(&self, task_id: Uuid) -> Result<(), Error> {
        let request = TaskBatchLifecycleRequest {
            task_ids: vec![task_id],
        };
        self.batch_lifecycle_one(
            "/v1/tasks/batch/purge",
            &request,
            task_id,
            "task must be trashed before permanent deletion",
        )
        .await
    }

    pub async fn cancel_active_tasks(&self) -> Result<CancelActiveTasksResponse, Error> {
        self.execute_json(
            self.authorized_request(Method::POST, "/v1/admin/tasks/cancel-active")
                .await?,
        )
        .await
    }

    /// Reads the durable global admission state of the task queue
    /// (`GET /v1/admin/tasks/queue`).
    ///
    /// A pause stops new claims across every replica; already claimed work keeps
    /// running.
    pub async fn task_queue_state(&self) -> Result<TaskQueueStateResponse, Error> {
        self.execute_json(
            self.authorized_request(Method::GET, "/v1/admin/tasks/queue")
                .await?,
        )
        .await
    }

    /// Writes the durable global admission state of the task queue
    /// (`PUT /v1/admin/tasks/queue`).
    ///
    /// The write commits before it answers, so a pause survives a restart. A
    /// resume wakes the receiving replica's dispatch immediately; another
    /// replica picks the queue up on its recovery tick.
    pub async fn update_task_queue_state(
        &self,
        paused: bool,
    ) -> Result<TaskQueueStateResponse, Error> {
        self.execute_json(
            self.authorized_request(Method::PUT, "/v1/admin/tasks/queue")
                .await?
                .json(&UpdateTaskQueueStateRequest { paused }),
        )
        .await
    }

    /// Reads the durable status of the vector index rebuild
    /// (`GET /v1/settings/runtime/vector-index/rebuild`).
    ///
    /// The status names the task the controls act on, so a control can never
    /// touch a rebuild the caller did not observe.
    pub async fn vector_index_rebuild_status(&self) -> Result<VectorIndexRebuildStatus, Error> {
        self.execute_json(
            self.authorized_request(Method::GET, "/v1/settings/runtime/vector-index/rebuild")
                .await?,
        )
        .await
    }

    /// Pauses the rebuild the observed task owns
    /// (`POST /v1/settings/runtime/vector-index/rebuild/pause`).
    ///
    /// The pause takes effect at the next batch boundary: the committed page
    /// still finishes.
    pub async fn pause_vector_index_rebuild(
        &self,
        task_id: Uuid,
    ) -> Result<VectorIndexRebuildStatus, Error> {
        self.execute_json(
            self.authorized_request(
                Method::POST,
                "/v1/settings/runtime/vector-index/rebuild/pause",
            )
            .await?
            .json(&RebuildControlRequest { task_id }),
        )
        .await
    }

    /// Resumes the rebuild the observed task owns
    /// (`POST /v1/settings/runtime/vector-index/rebuild/resume`).
    ///
    /// The rebuild continues after its last written page.
    pub async fn resume_vector_index_rebuild(
        &self,
        task_id: Uuid,
    ) -> Result<VectorIndexRebuildStatus, Error> {
        self.execute_json(
            self.authorized_request(
                Method::POST,
                "/v1/settings/runtime/vector-index/rebuild/resume",
            )
            .await?
            .json(&RebuildControlRequest { task_id }),
        )
        .await
    }

    /// Cancels the rebuild the observed task owns
    /// (`POST /v1/settings/runtime/vector-index/rebuild/cancel`).
    ///
    /// The cancelled attempt cannot continue; the existing collections are
    /// kept.
    pub async fn cancel_vector_index_rebuild(
        &self,
        task_id: Uuid,
    ) -> Result<VectorIndexRebuildStatus, Error> {
        self.execute_json(
            self.authorized_request(
                Method::POST,
                "/v1/settings/runtime/vector-index/rebuild/cancel",
            )
            .await?
            .json(&RebuildControlRequest { task_id }),
        )
        .await
    }

    /// Bounded admin read of remote Docling task identities
    /// (`GET /v1/admin/docling/tasks`).
    ///
    /// Issue 84 P1: `limit` is `1..=100`, `offset` is `>=0`; the fork's
    /// `has_more` is returned verbatim and no total is invented. Uses the
    /// saved Docling endpoint; unknown `task_status` values stay text.
    pub async fn list_docling_remote_tasks(
        &self,
        query: &DoclingRemoteTasksQuery,
    ) -> Result<DoclingRemoteTasksResponse, Error> {
        query.validate().map_err(|error| {
            Error::InvalidResponse(format!("invalid docling remote tasks query: {error}"))
        })?;
        self.execute_json(
            self.authorized_request(Method::GET, "/v1/admin/docling/tasks")
                .await?
                .query(query),
        )
        .await
    }

    pub async fn wake_docling_cleanup(
        &self,
    ) -> Result<context69_contracts::DoclingCleanupResponse, Error> {
        self.execute_json(
            self.authorized_request(Method::POST, "/v1/admin/docling/cleanup")
                .await?,
        )
        .await
    }

    /// Canonical `search` (`POST /v1/search`). Returns the full
    /// [`SearchResponse`] including [`SearchPagination`] so callers keep
    /// `next_cursor`/`has_more` continuation metadata.
    pub async fn search(&self, request: &SearchRequest) -> Result<SearchResponse, Error> {
        self.execute_json(
            self.authorized_request(Method::POST, "/v1/search")
                .await?
                .json(request),
        )
        .await
    }

    /// Bounded compact projection over `search`. Snippets are truncated to
    /// 320 chars, but pagination is never dropped: the returned
    /// [`CompactSearchResponse`] preserves the full [`SearchPagination`]
    /// (`next_cursor`/`has_more`) from the underlying [`SearchResponse`].
    pub async fn search_compact(
        &self,
        request: &SearchRequest,
    ) -> Result<CompactSearchResponse, Error> {
        let response: SearchResponse = self.search(request).await?;
        Ok(CompactSearchResponse {
            query: response.query,
            hits: response
                .items
                .into_iter()
                .map(|hit| CompactSearchHit {
                    document_id: hit.document_id,
                    external_id: hit.external_id,
                    title: hit.title,
                    summary: hit.summary,
                    source_uri: hit.source_uri,
                    published_at: hit.published_at,
                    score: hit.score,
                    snippet: hit.chunk_text.chars().take(320).collect(),
                })
                .collect(),
            pagination: response.pagination,
        })
    }

    pub async fn get_document(
        &self,
        document_id: i64,
        locale: Option<&str>,
    ) -> Result<DocumentResponse, Error> {
        let path = format!("/v1/documents/{document_id}");
        self.execute_json(
            self.authorized_request(Method::GET, &path)
                .await?
                .query(&[("locale", locale.unwrap_or("").to_string())]),
        )
        .await
    }

    pub async fn get_document_by_key(
        &self,
        group_path_value: &str,
        key: &DocumentKey,
        locale: Option<&str>,
    ) -> Result<DocumentResponse, Error> {
        let path = group_path(group_path_value, "/documents/by-external-id");
        self.execute_json(self.authorized_request(Method::GET, &path).await?.query(&[
            ("source_key", key.source_key.clone()),
            ("external_id", key.external_id.clone()),
            ("locale", locale.unwrap_or("").to_string()),
        ]))
        .await
    }

    pub async fn get_documents(
        &self,
        group_path_value: &str,
        request: &BatchGetDocumentsRequest,
    ) -> Result<BatchGetDocumentsResponse, Error> {
        let path = group_path(group_path_value, "/documents/batch-get");
        self.execute_json(
            self.authorized_request(Method::POST, &path)
                .await?
                .json(request),
        )
        .await
    }

    /// Append one caller-produced result to a document
    /// (`POST /v1/groups/by-path/{group_path}/documents/{document_id}/results`).
    ///
    /// Append-only: an exact retry returns the original immutable record, and a
    /// changed envelope under a used idempotency key is a `409`. The caller must
    /// bind the document's current `record_hash` as `source_record_hash`.
    pub async fn create_document_result(
        &self,
        group_path_value: &str,
        document_id: i64,
        request: &CreateDocumentResultRequest,
    ) -> Result<DocumentResultResponse, Error> {
        let path = format!(
            "{}/documents/{document_id}/results",
            group_path(group_path_value, "")
        );
        self.execute_json(
            self.authorized_request(Method::POST, &path)
                .await?
                .json(request),
        )
        .await
    }

    /// List one document's appended results newest-first
    /// (`GET /v1/groups/by-path/{group_path}/documents/{document_id}/results`).
    ///
    /// `limit` is validated (`1..=100`); `producer`/`result_kind`/`after_id` are
    /// omitted when unset. `current_only` restricts the page to the document's
    /// current source version and `latest_only` keeps the newest row per
    /// `(producer, result_kind)`. The response carries the document's current
    /// `source_record_hash` for binding a subsequent append.
    pub async fn list_document_results(
        &self,
        group_path_value: &str,
        document_id: i64,
        query: &ListDocumentResultsQuery,
    ) -> Result<ListDocumentResultsResponse, Error> {
        query
            .validate()
            .map_err(|error| Error::InvalidResponse(format!("invalid result query: {error}")))?;
        let path = format!(
            "{}/documents/{document_id}/results",
            group_path(group_path_value, "")
        );
        let mut pairs: Vec<(String, String)> = vec![("limit".to_string(), query.limit.to_string())];
        if let Some(producer) = &query.producer {
            pairs.push(("producer".to_string(), producer.clone()));
        }
        if let Some(result_kind) = &query.result_kind {
            pairs.push(("result_kind".to_string(), result_kind.clone()));
        }
        if let Some(after_id) = query.after_id {
            pairs.push(("after_id".to_string(), after_id.to_string()));
        }
        if query.current_only {
            pairs.push(("current_only".to_string(), "true".to_string()));
        }
        if query.latest_only {
            pairs.push(("latest_only".to_string(), "true".to_string()));
        }
        self.execute_json(
            self.authorized_request(Method::GET, &path)
                .await?
                .query(&pairs),
        )
        .await
    }

    pub async fn me(&self) -> Result<AuthMeResponse, Error> {
        self.execute_json(self.authorized_request(Method::GET, "/v1/auth/me").await?)
            .await
    }

    pub async fn healthz(&self) -> Result<HealthResponse, Error> {
        let response = self.client.get(self.url("/healthz")?).send().await?;
        self.read_json_response(response).await
    }
}

fn idempotency_key<T: serde::Serialize>(path: &str, body: &T) -> Result<String, Error> {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(path.as_bytes());
    hasher.update([0]);
    hasher.update(serde_json::to_vec(body)?);
    Ok(format!(
        "ctx69-sdk-{}",
        hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    ))
}

#[cfg(test)]
mod tests {
    use reqwest::StatusCode;

    use super::{canonical_refusal, classify_masked_read, idempotency_key};
    use crate::Error;

    fn assert_refusal(error: Error, status: StatusCode, code: &str, message: &str) {
        match error {
            Error::HttpStatus {
                status: actual,
                api_error,
                body,
            } => {
                assert_eq!(actual, status);
                assert_eq!(api_error.as_deref(), Some(message));
                let parsed: serde_json::Value =
                    serde_json::from_str(&body).expect("canonical error body");
                assert_eq!(parsed["code"], code);
                assert_eq!(parsed["message"], message);
            }
            other => panic!("expected an HTTP-status refusal, got {other:?}"),
        }
    }

    #[test]
    fn singleton_refusals_reproduce_the_legacy_error_contract() {
        // Readable but unmanageable: the batch masks it as not found, the
        // read-only follow-up distinguishes the legacy 403.
        assert_refusal(
            classify_masked_read(Ok(())),
            StatusCode::FORBIDDEN,
            "forbidden",
            "task management permission denied",
        );
        // Genuinely missing: the follow-up read is also 404.
        assert_refusal(
            classify_masked_read::<()>(Err(Error::HttpStatus {
                status: StatusCode::NOT_FOUND,
                api_error: Some("task not found".to_string()),
                body: String::new(),
            })),
            StatusCode::NOT_FOUND,
            "not_found",
            "task not found",
        );
        // Non-404 read failures propagate unchanged.
        match classify_masked_read::<()>(Err(Error::InvalidResponse("boom".to_string()))) {
            Error::InvalidResponse(message) => assert_eq!(message, "boom"),
            other => panic!("expected the read error to propagate, got {other:?}"),
        }

        assert_refusal(
            canonical_refusal(
                StatusCode::CONFLICT,
                "active task cannot be trashed until it reaches a terminal state",
            ),
            StatusCode::CONFLICT,
            "conflict",
            "active task cannot be trashed until it reaches a terminal state",
        );
        assert_refusal(
            canonical_refusal(
                StatusCode::INTERNAL_SERVER_ERROR,
                "task lifecycle action failed",
            ),
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "task lifecycle action failed",
        );
    }

    #[test]
    fn batch_idempotency_key_is_stable_for_the_same_request() {
        let first = idempotency_key(
            "/v1/groups/by-path/research/batch/text",
            &serde_json::json!({"items":[{"external_id":"a"}]}),
        )
        .expect("key");
        let second = idempotency_key(
            "/v1/groups/by-path/research/batch/text",
            &serde_json::json!({"items":[{"external_id":"a"}]}),
        )
        .expect("key");
        let different = idempotency_key(
            "/v1/groups/by-path/research/batch/text",
            &serde_json::json!({"items":[{"external_id":"b"}]}),
        )
        .expect("key");
        assert_eq!(first, second);
        assert_ne!(first, different);
    }
}
