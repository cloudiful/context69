use std::time::{Duration, Instant};

pub use context69_contracts::{
    AuthMeResponse, BatchGetDocumentsRequest, BatchGetDocumentsResponse, CancelActiveTasksResponse,
    CreateMetadataIndexRequest, DeleteBatchRequest, DocumentChunkResponse, DocumentKey,
    DocumentResponse, EnsureScopeResponse, FileBatchItem, FileBatchRequest, GroupKind,
    GroupResponse, HealthResponse, ImportLibraryFileFromUrlRequest as UrlBatchItem,
    LibraryFileUploadMetadata as FileMetadata, LibraryTextContentFormat as TextContentFormat,
    MetadataDataType, MetadataFilter, MetadataFilterOperator, MetadataValueKind, PurgeTasksRequest,
    PurgeTasksResponse, RerunTaskResponse, ScopeMetadataIndex, ScopeSpec, SearchRequest,
    TaskItemResponse, TaskItemStatus, TaskItemsResponse, TaskKind, TaskListQuery,
    TaskMaintenanceOverview, TaskPageResponse, TaskProgress, TaskRef, TaskResponse,
    TaskRetryResponse, TaskStatus, TaskSubmitRequest, TextBatchRequest, TranslationDirective,
    TranslationStatus, UpdateTaskMaintenanceSettingsRequest,
    UpsertLibraryTextRequest as TextBatchItem, UrlBatchRequest, Visibility,
};
use reqwest::Method;
use uuid::Uuid;

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

    pub async fn text_batch(
        &self,
        group_path_value: &str,
        request: &TextBatchRequest,
    ) -> Result<TaskRef, Error> {
        self.submit_batch(group_path(group_path_value, "/batch/text"), request)
            .await
    }

    pub async fn url_batch(
        &self,
        group_path_value: &str,
        request: &UrlBatchRequest,
    ) -> Result<TaskRef, Error> {
        self.submit_batch(group_path(group_path_value, "/batch/url"), request)
            .await
    }

    pub async fn file_batch(
        &self,
        group_path_value: &str,
        request: &FileBatchRequest,
    ) -> Result<TaskRef, Error> {
        self.submit_batch(group_path(group_path_value, "/batch/file"), request)
            .await
    }

    pub async fn delete_batch(
        &self,
        group_path_value: &str,
        request: &DeleteBatchRequest,
    ) -> Result<TaskRef, Error> {
        self.submit_batch(group_path(group_path_value, "/batch/delete"), request)
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

    pub async fn task(&self, task_id: Uuid) -> Result<TaskResponse, Error> {
        let path = format!("/v1/tasks/{task_id}");
        self.execute_json(self.authorized_request(Method::GET, &path).await?)
            .await
    }

    pub async fn tasks(&self, query: &TaskListQuery) -> Result<TaskPageResponse, Error> {
        self.execute_json(
            self.authorized_request(Method::GET, "/v1/tasks")
                .await?
                .query(query),
        )
        .await
    }

    pub async fn task_items(
        &self,
        task_id: Uuid,
        cursor: Option<&str>,
    ) -> Result<TaskItemsResponse, Error> {
        let path = format!("/v1/tasks/{task_id}/items");
        let query = [
            ("limit", "200".to_string()),
            ("cursor", cursor.unwrap_or("0").to_string()),
        ];
        self.execute_json(
            self.authorized_request(Method::GET, &path)
                .await?
                .query(&query),
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
            match self.task(task_id).await {
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

    pub async fn search_compact(
        &self,
        request: &SearchRequest,
    ) -> Result<CompactSearchResponse, Error> {
        let response: context69_contracts::SearchResponse = self
            .execute_json(
                self.authorized_request(Method::POST, "/v1/search")
                    .await?
                    .json(request),
            )
            .await?;
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
