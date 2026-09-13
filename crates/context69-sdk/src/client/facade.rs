use std::time::{Duration, Instant};

pub use context69_contracts::{
    AuthMeResponse, AuthUserResponse, BatchGetDocumentsRequest, BatchGetDocumentsResponse,
    CancelActiveTasksResponse, CanonicalSearchRequest, CanonicalTaskListQuery,
    CanonicalUpdateSearchSettingsRequest, CanonicalUploadMetadata, CreateMetadataIndexRequest,
    CursorPageQuery, CursorPagination, DeleteBatchRequest, DocumentChunkResponse, DocumentKey,
    DocumentResponse, EnsureScopeResponse, ExtractionDirective, ExtractionJobsResponse,
    ExtractionTemplateInput, ExtractionTemplateResponse, FileBatchItem, FileBatchRequest,
    GroupKind, GroupResponse, HealthResponse, ImportLibraryFileFromUrlRequest as UrlBatchItem,
    IngestOptions, LibraryFileDetailResponse, LibraryFileUploadMetadata as FileMetadata,
    LibraryTextContentFormat as TextContentFormat, MetadataDataType, MetadataFilter,
    MetadataFilterOperator, MetadataValueKind, OffsetPageQuery, OffsetPagination,
    PurgeTasksRequest, PurgeTasksResponse, RebuildDocumentExtractionsRequest, RerunTaskResponse,
    ScopeMetadataIndex, ScopeSpec, SearchMode, SearchRequest, SearchResponse, SearchSort,
    SecretPatch, SortDirection, SourcePolicy, TaskItemResponse, TaskItemStatus, TaskItemsQuery,
    TaskItemsResponse, TaskKind, TaskListQuery, TaskListView, TaskMaintenanceOverview,
    TaskMaintenanceSettings, TaskPageResponse, TaskProgress, TaskPurgeMode, TaskRef, TaskResponse,
    TaskRetryResponse, TaskSortBy, TaskStatus, TaskSubmitRequest, TextBatchRequest,
    TranslationDirective, TranslationStatus, UpdateTaskMaintenanceSettingsRequest,
    UpsertLibraryTextRequest as TextBatchItem, UrlBatchRequest, Visibility,
};
use reqwest::Method;
use uuid::Uuid;

pub use context69_contracts::search::SearchPagination;

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
/// and is omitted when `None` (no hidden `cursor=0` default).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TaskItemsOptions {
    pub limit: u32,
    pub cursor: Option<String>,
}

impl Default for TaskItemsOptions {
    fn default() -> Self {
        Self {
            limit: 100,
            cursor: None,
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

    #[deprecated(note = "use submit_text_batch instead")]
    pub async fn text_batch(
        &self,
        group_path_value: &str,
        request: &TextBatchRequest,
    ) -> Result<TaskRef, Error> {
        self.submit_batch(group_path(group_path_value, "/batch/text"), request)
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

    #[deprecated(note = "use submit_url_batch instead")]
    pub async fn url_batch(
        &self,
        group_path_value: &str,
        request: &UrlBatchRequest,
    ) -> Result<TaskRef, Error> {
        self.submit_batch(group_path(group_path_value, "/batch/url"), request)
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

    #[deprecated(note = "use submit_file_batch instead")]
    pub async fn file_batch(
        &self,
        group_path_value: &str,
        request: &FileBatchRequest,
    ) -> Result<TaskRef, Error> {
        self.submit_batch(group_path(group_path_value, "/batch/file"), request)
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

    #[deprecated(note = "use submit_delete_batch instead")]
    pub async fn delete_batch(
        &self,
        group_path_value: &str,
        request: &DeleteBatchRequest,
    ) -> Result<TaskRef, Error> {
        self.submit_batch(group_path(group_path_value, "/batch/delete"), request)
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

    /// Deprecated alias for [`Self::get_task`]; retained for v0.15 callers.
    #[deprecated(note = "use get_task instead")]
    pub async fn task(&self, task_id: Uuid) -> Result<TaskResponse, Error> {
        self.get_task(task_id).await
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

    /// Deprecated v0.15 list shape (`GET /v1/tasks` with [`TaskListQuery`]).
    /// New code should use [`Self::list_tasks`] with [`TaskListOptions`],
    /// which requires a typed `view` and validates bounds client-side.
    #[deprecated(note = "use list_tasks with TaskListOptions instead")]
    pub async fn tasks(&self, query: &TaskListQuery) -> Result<TaskPageResponse, Error> {
        self.execute_json(
            self.authorized_request(Method::GET, "/v1/tasks")
                .await?
                .query(query),
        )
        .await
    }

    /// Canonical `list_task_items` (`GET /v1/tasks/{task_id}/items`).
    /// `limit` is validated (`1..=100`); `cursor` is omitted when `None`
    /// so the server applies its own default instead of a hidden `0`.
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
        self.execute_json(
            self.authorized_request(Method::GET, &path)
                .await?
                .query(&pairs),
        )
        .await
    }

    /// Deprecated v0.15 item window. It no longer hides `limit=200` or
    /// `cursor=0`: it delegates to [`Self::list_task_items`] with the wire
    /// default (`limit=100`) and omits the cursor when `None`.
    #[deprecated(note = "use list_task_items with TaskItemsOptions instead")]
    pub async fn task_items(
        &self,
        task_id: Uuid,
        cursor: Option<&str>,
    ) -> Result<TaskItemsResponse, Error> {
        self.list_task_items(
            task_id,
            &TaskItemsOptions {
                limit: 100,
                cursor: cursor.map(str::to_string),
            },
        )
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

    pub async fn rerun_task(&self, task_id: Uuid) -> Result<RerunTaskResponse, Error> {
        let path = format!("/v1/tasks/{task_id}/rerun");
        self.execute_json(self.authorized_request(Method::POST, &path).await?)
            .await
    }

    pub async fn cancel_task(&self, task_id: Uuid) -> Result<(), Error> {
        let path = format!("/v1/tasks/{task_id}/cancel");
        self.execute_empty(self.authorized_request(Method::POST, &path).await?)
            .await
    }

    /// Move a terminal task's history into the recycle bin. Idempotent and
    /// rejected while the task is still active. Files and processed results
    /// are never affected.
    pub async fn trash_task(&self, task_id: Uuid) -> Result<TaskResponse, Error> {
        let path = format!("/v1/tasks/{task_id}/trash");
        self.execute_json(self.authorized_request(Method::POST, &path).await?)
            .await
    }

    /// Restore a trashed task's history. Idempotent.
    pub async fn restore_task(&self, task_id: Uuid) -> Result<TaskResponse, Error> {
        let path = format!("/v1/tasks/{task_id}/restore");
        self.execute_json(self.authorized_request(Method::POST, &path).await?)
            .await
    }

    /// Permanently delete a trashed task's history (`DELETE /v1/tasks/{task_id}`).
    /// Only trashed rows qualify; the call is idempotent for an already-purged id
    /// from the caller's perspective (server returns 404 when nothing remains).
    pub async fn purge_trashed_task(&self, task_id: Uuid) -> Result<(), Error> {
        let path = format!("/v1/tasks/{task_id}");
        self.execute_empty(self.authorized_request(Method::DELETE, &path).await?)
            .await
    }

    /// Deprecated alias for [`Self::purge_trashed_task`]; the operation only
    /// ever deleted trashed task history.
    #[deprecated(note = "use purge_trashed_task instead")]
    pub async fn delete_task(&self, task_id: Uuid) -> Result<(), Error> {
        self.purge_trashed_task(task_id).await
    }

    pub async fn task_maintenance(&self) -> Result<TaskMaintenanceOverview, Error> {
        self.execute_json(
            self.authorized_request(Method::GET, "/v1/admin/tasks/maintenance")
                .await?,
        )
        .await
    }

    pub async fn update_task_maintenance(
        &self,
        request: &UpdateTaskMaintenanceSettingsRequest,
    ) -> Result<TaskMaintenanceOverview, Error> {
        self.execute_json(
            self.authorized_request(Method::PUT, "/v1/admin/tasks/maintenance")
                .await?
                .json(request),
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

    pub async fn purge_tasks(
        &self,
        request: &PurgeTasksRequest,
    ) -> Result<PurgeTasksResponse, Error> {
        self.execute_json(
            self.authorized_request(Method::POST, "/v1/admin/tasks/purge")
                .await?
                .json(request),
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

    pub async fn list_extraction_templates(
        &self,
        group_path_value: &str,
    ) -> Result<Vec<ExtractionTemplateResponse>, Error> {
        let path = group_path(group_path_value, "/extraction-templates");
        self.execute_json(self.authorized_request(Method::GET, &path).await?)
            .await
    }

    pub async fn upsert_extraction_template(
        &self,
        group_path_value: &str,
        request: &ExtractionTemplateInput,
    ) -> Result<ExtractionTemplateResponse, Error> {
        let path = group_path(group_path_value, "/extraction-templates");
        self.execute_json(
            self.authorized_request(Method::PUT, &path)
                .await?
                .json(request),
        )
        .await
    }

    pub async fn document_extractions(
        &self,
        group_path_value: &str,
        document_id: i64,
    ) -> Result<ExtractionJobsResponse, Error> {
        let path = format!(
            "{}/documents/{document_id}/extractions",
            group_path(group_path_value, "")
        );
        self.execute_json(self.authorized_request(Method::GET, &path).await?)
            .await
    }

    pub async fn rebuild_document_extractions(
        &self,
        group_path_value: &str,
        document_id: i64,
        request: &RebuildDocumentExtractionsRequest,
    ) -> Result<ExtractionJobsResponse, Error> {
        let path = format!(
            "{}/documents/{document_id}/extractions/rebuild",
            group_path(group_path_value, "")
        );
        self.execute_json(
            self.authorized_request(Method::POST, &path)
                .await?
                .json(request),
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
    use super::idempotency_key;

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
