use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Context, Result, anyhow};
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::Utc;
use context69_contracts::{
    CreateGroupRequest, CreateMetadataIndexRequest, EnsureScopeResponse, ExternalJobInfo,
    FileBatchItem, GroupResponse, MetadataIndexResponse, MetadataIndexStatus, RerunTaskResponse,
    ScopeSpec, SortDirection, TaskItemResponse, TaskItemStatus, TaskItemsResponse, TaskKind,
    TaskListQuery, TaskOrigin, TaskPageResponse, TaskProgress, TaskRef, TaskResponse,
    TaskRetryResponse, TaskSortBy, TaskStatus,
};
use context69_translation::TranslationService;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::{
    sync::{Notify, OwnedSemaphorePermit, Semaphore},
    time::sleep,
};
use uuid::Uuid;

use crate::{
    db::{Database, StoredTask, StoredTaskItemWithExternalJob},
    domain::GroupRecord,
    pagination::PageBounds,
    services::{
        document_store::DocumentStoreService, library::LibraryService, namespace::NamespaceService,
        source_folders::SourceFoldersService, sync::SyncService,
    },
};

mod dispatcher;
mod item_file_processors;
mod item_lifecycle_processors;
mod item_processors;
mod item_translation_processors;
mod item_url_processor;
mod maintenance;
mod runtime;

#[derive(Clone)]
pub struct TaskService {
    db: Database,
    namespace: NamespaceService,
    document_store: DocumentStoreService,
    library: LibraryService,
    sync: SyncService,
    source_folders: SourceFoldersService,
    translation: TranslationService,
    worker_slots: Arc<Semaphore>,
    worker_capacity: usize,
    dispatch_notify: Arc<Notify>,
    dispatcher_started: Arc<AtomicBool>,
}

#[derive(Debug, Clone)]
pub struct TaskSubmission {
    pub user_id: i64,
    pub group_id: Option<i64>,
    pub group_path: Option<String>,
    pub source_key: Option<String>,
    pub kind: TaskKind,
    pub payloads: Vec<Value>,
    pub input_storage_object_ids: Vec<Option<Uuid>>,
    pub idempotency_key: Option<String>,
}

impl TaskService {
    pub fn new(
        db: Database,
        namespace: NamespaceService,
        document_store: DocumentStoreService,
        library: LibraryService,
        sync: SyncService,
        source_folders: SourceFoldersService,
        translation: TranslationService,
        concurrency: usize,
    ) -> Self {
        let worker_capacity = concurrency.max(1);
        Self {
            db,
            namespace,
            document_store,
            library,
            sync,
            source_folders,
            translation,
            worker_slots: Arc::new(Semaphore::new(worker_capacity)),
            worker_capacity,
            dispatch_notify: Arc::new(Notify::new()),
            dispatcher_started: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn resume_pending(&self) {
        dispatcher::start(self);
    }

    pub fn start_maintenance(&self) {
        maintenance::start(self);
    }

    pub async fn submit(&self, request: TaskSubmission) -> Result<TaskRef> {
        if request.payloads.is_empty() {
            return Err(anyhow!("a task must contain at least one item"));
        }
        let request_hash = hash_payload(&request);
        let mut payloads = request.payloads.clone();
        let mut input_storage_object_ids = if request.input_storage_object_ids.is_empty() {
            vec![None; payloads.len()]
        } else {
            request.input_storage_object_ids.clone()
        };
        if input_storage_object_ids.len() != payloads.len() {
            return Err(anyhow!("task payload and input object counts do not match"));
        }
        let mut newly_staged_object_ids = Vec::new();
        if request.kind == TaskKind::FileBatch {
            let group_id = request.group_id.context("file tasks require group_id")?;
            let result = async {
                for (index, payload) in payloads.iter_mut().enumerate() {
                    if payload.get("file_id").is_some() || payload.get("content_base64").is_none() {
                        continue;
                    }
                    let file: FileBatchItem = serde_json::from_value(payload.clone())
                        .context("invalid file batch item")?;
                    let bytes = STANDARD
                        .decode(file.content_base64.trim())
                        .context("invalid file batch content_base64")?;
                    let object_id = self
                        .library
                        .stage_file_for_task_input(
                            group_id,
                            crate::services::library::UploadedLibraryFile {
                                folder_id: file.folder_id,
                                filename: file.filename,
                                media_type: file.media_type,
                                bytes: bytes.into(),
                                declared_sha256: file.declared_sha256,
                                metadata: file.metadata,
                                translation: file.translation,
                                extraction: file.extraction,
                                staged_storage_object_id: None,
                            },
                        )
                        .await?;
                    input_storage_object_ids[index] = Some(object_id);
                    newly_staged_object_ids.push(object_id);
                    if let Some(object) = payload.as_object_mut() {
                        object.remove("content_base64");
                    }
                }
                Ok::<_, anyhow::Error>(())
            }
            .await;
            if let Err(error) = result {
                self.release_staged_input_objects(&newly_staged_object_ids)
                    .await;
                return Err(error);
            }
        }
        let key = request
            .idempotency_key
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty());
        let task_id = Uuid::new_v4();
        let submission = self
            .db
            .create_task_submission_with_input_objects(
                task_id,
                request.user_id,
                request.group_id,
                request.kind.as_str(),
                request.group_path.as_deref(),
                request.source_key.as_deref(),
                &payloads,
                &input_storage_object_ids,
                key,
                &request_hash,
            )
            .await;
        let (task_id, reused, item_ids) = match submission {
            Ok(value) => value,
            Err(error) => {
                self.release_staged_input_objects(&newly_staged_object_ids)
                    .await;
                return Err(error);
            }
        };
        if reused {
            self.release_staged_input_objects(&newly_staged_object_ids)
                .await;
        }
        if !reused {
            self.notify_dispatch();
        }
        Ok(TaskRef { task_id, item_ids })
    }

    async fn release_staged_input_objects(&self, object_ids: &[Uuid]) {
        for &object_id in object_ids {
            if let Err(error) = self
                .library
                .release_task_input_staging(object_id, None)
                .await
            {
                tracing::warn!(%object_id, %error, "failed to release staged task input after submission failure");
            }
        }
    }

    pub async fn get(&self, task_id: Uuid, user_id: i64) -> Result<TaskResponse> {
        self.db
            .get_task(task_id, user_id)
            .await?
            .context("task not found")
            .map(task_response)
    }

    pub async fn list(&self, user_id: i64, query: &TaskListQuery) -> Result<TaskPageResponse> {
        let bounds = PageBounds::new(query.page, query.page_size)?;
        let kind = query.kind.map(TaskKind::as_str);
        let status = query.status.map(TaskStatus::as_str);
        let total = self
            .db
            .count_tasks(
                user_id,
                query.query.as_deref(),
                kind,
                status,
                query.stage.as_deref(),
                query.waiting_reason.as_deref(),
                query.dependency_key.as_deref(),
            )
            .await?;
        let items = self
            .db
            .list_tasks(
                user_id,
                query.query.as_deref(),
                kind,
                status,
                query.stage.as_deref(),
                query.waiting_reason.as_deref(),
                query.dependency_key.as_deref(),
                query.sort_by.map(TaskSortBy::as_str),
                query.sort_direction.map(SortDirection::as_str),
                i64::from(bounds.page_size),
                bounds.offset,
            )
            .await?
            .into_iter()
            .map(task_response)
            .collect();
        Ok(TaskPageResponse {
            items,
            pagination: bounds.pagination(total)?,
        })
    }

    pub async fn items(
        &self,
        task_id: Uuid,
        user_id: i64,
        limit: i64,
        offset: i64,
    ) -> Result<TaskItemsResponse> {
        self.db
            .get_task(task_id, user_id)
            .await?
            .context("task not found")?;
        let limit = limit.clamp(1, 200);
        let offset = offset.max(0);
        let items = self.db.list_task_items(task_id, limit, offset).await?;
        let next_cursor =
            (items.len() as i64 == limit).then(|| (offset + items.len() as i64).to_string());
        Ok(TaskItemsResponse {
            items: items.into_iter().map(task_item_response).collect(),
            next_cursor,
        })
    }

    pub async fn retry(&self, task_id: Uuid, user_id: i64) -> Result<TaskRetryResponse> {
        self.db
            .get_task(task_id, user_id)
            .await?
            .context("task not found")?;
        if !self.db.can_manage_task(task_id, user_id).await? {
            return Err(anyhow!("task management permission denied"));
        }
        let item_ids = self.db.retry_task_items(task_id, user_id).await?;
        if item_ids.is_empty() {
            return Err(anyhow!("task has no retryable failed items"));
        }
        self.db.recompute_task(task_id).await?;
        self.notify_dispatch();
        Ok(TaskRetryResponse {
            task: TaskRef {
                task_id,
                item_ids: item_ids.clone(),
            },
            retried_items: item_ids.len() as i64,
        })
    }

    pub async fn cancel(&self, task_id: Uuid, user_id: i64) -> Result<()> {
        self.db
            .get_task(task_id, user_id)
            .await?
            .context("task not found")?;
        if !self.db.can_manage_task(task_id, user_id).await? {
            return Err(anyhow!("task management permission denied"));
        }
        if self.db.cancel_task(task_id, user_id).await? {
            Ok(())
        } else {
            Err(anyhow!("task is already terminal or not found"))
        }
    }

    pub async fn rerun(&self, task_id: Uuid, user_id: i64) -> Result<RerunTaskResponse> {
        self.db
            .get_task(task_id, user_id)
            .await?
            .context("task not found")?;
        if !self.db.can_manage_task(task_id, user_id).await? {
            return Err(anyhow!("task management permission denied"));
        }
        let source = self
            .db
            .get_task_internal(task_id)
            .await?
            .context("task not found")?;
        if !matches!(source.status.as_str(), "cancelled" | "failed") {
            return Err(anyhow!(
                "task must be cancelled or failed before it can be rerun"
            ));
        }
        let (new_task_id, item_ids) = self.db.rerun_task(task_id).await?;
        if !item_ids.is_empty() {
            self.notify_dispatch();
        }
        tracing::info!(
            source_task_id = %task_id,
            rerun_task_id = %new_task_id,
            item_count = item_ids.len(),
            "context69 task rerun created"
        );
        Ok(RerunTaskResponse {
            task: TaskRef {
                task_id: new_task_id,
                item_ids,
            },
        })
    }

    pub async fn ensure_scope(
        &self,
        user_id: i64,
        spec: &ScopeSpec,
    ) -> Result<EnsureScopeResponse> {
        let actor = self
            .db
            .get_user_by_id(user_id)
            .await?
            .context("user not found")?;
        let (parent_group_path, group_key) = split_scope_path(&spec.group_path)?;
        let group = match self
            .namespace
            .get_group_for_user(user_id, &spec.group_path)
            .await?
        {
            Some(group) => group,
            None => match self
                .namespace
                .create_group(
                    &actor,
                    &CreateGroupRequest {
                        parent_group_path,
                        group_key,
                        name: spec.name.clone(),
                        visibility: spec.visibility,
                        kind: spec.kind,
                    },
                )
                .await
            {
                Ok(group) => group,
                Err(error) if is_conflict_error(&error) => self
                    .namespace
                    .get_group_for_user(user_id, &spec.group_path)
                    .await?
                    .context("scope creation conflicted")?,
                Err(error) => return Err(error),
            },
        };
        ensure_group_definition(&group, spec)?;
        let mut indexes = Vec::new();
        for requested in &spec.metadata_indexes {
            let existing = self
                .document_store
                .list_indexes(group.id, &requested.source_key)
                .await?
                .into_iter()
                .find(|index| index.path == requested.definition.path);
            match existing {
                Some(index) => {
                    ensure_index_definition(&index, &requested.definition)?;
                    index
                }
                None => match self
                    .document_store
                    .create_index(
                        group.id,
                        &group.group_path,
                        &requested.source_key,
                        &requested.definition,
                    )
                    .await
                {
                    Ok(index) => index,
                    Err(error) if is_conflict_error(&error) => self
                        .document_store
                        .list_indexes(group.id, &requested.source_key)
                        .await?
                        .into_iter()
                        .find(|index| index.path == requested.definition.path)
                        .context("metadata index creation conflicted")?,
                    Err(error) => return Err(error),
                },
            };
            indexes.push(
                wait_for_index(
                    &self.document_store,
                    group.id,
                    &requested.source_key,
                    &requested.definition.path,
                )
                .await?,
            );
        }
        Ok(EnsureScopeResponse {
            group: group_response(group),
            metadata_indexes: indexes,
        })
    }

    fn notify_dispatch(&self) {
        self.dispatch_notify.notify_one();
    }

    pub(super) fn dispatcher_started(&self) -> bool {
        self.dispatcher_started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    pub(super) fn dispatch_notify(&self) -> &Notify {
        &self.dispatch_notify
    }

    pub(super) fn available_worker_slots(&self) -> usize {
        self.worker_slots.available_permits()
    }

    pub(super) fn worker_slots(&self) -> Arc<Semaphore> {
        Arc::clone(&self.worker_slots)
    }

    pub(super) fn worker_capacity(&self) -> usize {
        self.worker_capacity
    }

    pub(super) fn spawn_item(&self, item: crate::db::ClaimedItem, permit: OwnedSemaphorePermit) {
        let service = self.clone();
        tokio::spawn(async move {
            if let Err(error) = runtime::run_item(&service, item).await {
                tracing::warn!(%error, "context69 task item worker failed");
            }
            drop(permit);
            service.notify_dispatch();
        });
    }

    pub(crate) async fn task(&self, task_id: Uuid) -> Result<StoredTask> {
        self.db
            .get_task_internal(task_id)
            .await?
            .context("task disappeared")
    }
    pub(crate) fn db(&self) -> &Database {
        &self.db
    }
    pub(crate) fn library(&self) -> &LibraryService {
        &self.library
    }
    pub(crate) fn document_store(&self) -> &DocumentStoreService {
        &self.document_store
    }
    pub(crate) fn sync(&self) -> &SyncService {
        &self.sync
    }
    pub(crate) fn source_folders(&self) -> &SourceFoldersService {
        &self.source_folders
    }
    pub(crate) fn translation(&self) -> &TranslationService {
        &self.translation
    }
}

fn hash_payload(request: &TaskSubmission) -> String {
    let bytes = serde_json::to_vec(&(
        &request.kind,
        &request.group_id,
        &request.group_path,
        &request.source_key,
        &request.payloads,
    ))
    .expect("task payloads are serializable");
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn split_scope_path(path: &str) -> Result<(Option<String>, String)> {
    let mut parts = path
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let key = parts.pop().context("scope group_path must not be empty")?;
    if parts.iter().any(|part| part.len() > 100) || key.len() > 100 {
        return Err(anyhow!("scope path segment is too long"));
    }
    Ok((
        (!parts.is_empty()).then(|| parts.join("/")),
        key.to_string(),
    ))
}

fn ensure_group_definition(group: &GroupRecord, spec: &ScopeSpec) -> Result<()> {
    if group.name != spec.name
        || group.visibility != spec.visibility
        || spec.kind.is_some_and(|kind| group.kind != kind)
    {
        return Err(anyhow!("scope definition conflict for {}", spec.group_path));
    }
    Ok(())
}

fn ensure_index_definition(
    index: &MetadataIndexResponse,
    definition: &CreateMetadataIndexRequest,
) -> Result<()> {
    if index.data_type != definition.data_type
        || index.value_kind != definition.value_kind
        || index.sortable != definition.sortable
    {
        return Err(anyhow!(
            "metadata index definition conflict for {}",
            definition.path
        ));
    }
    Ok(())
}

async fn wait_for_index(
    service: &DocumentStoreService,
    group_id: i64,
    source_key: &str,
    path: &str,
) -> Result<MetadataIndexResponse> {
    for _ in 0..120 {
        if let Some(index) = service
            .list_indexes(group_id, source_key)
            .await?
            .into_iter()
            .find(|index| index.path == path)
        {
            match index.status {
                MetadataIndexStatus::Ready => return Ok(index),
                MetadataIndexStatus::Failed => {
                    return Err(anyhow!(
                        index
                            .error_message
                            .unwrap_or_else(|| "metadata index build failed".to_string())
                    ));
                }
                _ => sleep(Duration::from_millis(50)).await,
            }
        } else {
            sleep(Duration::from_millis(50)).await;
        }
    }
    Err(anyhow!("timed out waiting for metadata index {path}"))
}

fn parse_kind(value: &str) -> Result<TaskKind> {
    match value {
        "source_sync" => Ok(TaskKind::SourceSync),
        "text_batch" => Ok(TaskKind::TextBatch),
        "file_batch" => Ok(TaskKind::FileBatch),
        "url_batch" => Ok(TaskKind::UrlBatch),
        "delete_batch" => Ok(TaskKind::DeleteBatch),
        "translation" => Ok(TaskKind::Translation),
        "vector_rebuild" => Ok(TaskKind::VectorRebuild),
        other => Err(anyhow!("unsupported task kind {other}")),
    }
}

fn task_response(task: StoredTask) -> TaskResponse {
    let completed = task.succeeded_count + task.failed_count + task.cancelled_count;
    let eta_seconds = task
        .started_at
        .filter(|_| completed > 0 && completed < task.total_count)
        .map(|started_at| {
            let elapsed = (Utc::now() - started_at).num_seconds().max(1);
            (elapsed.saturating_mul(task.total_count - completed) / completed).max(1)
        });
    TaskResponse {
        task_id: task.id,
        kind: parse_kind(&task.kind).unwrap_or(TaskKind::TextBatch),
        status: parse_status(&task.status).unwrap_or(TaskStatus::Failed),
        origin: parse_origin(&task.origin).unwrap_or(TaskOrigin::Manual),
        group_path: task.group_path,
        source_key: task.source_key,
        progress: TaskProgress {
            total: task.total_count,
            queued: task.queued_count,
            running: task.running_count,
            waiting: task.waiting_count,
            succeeded: task.succeeded_count,
            failed: task.failed_count,
            cancelled: task.cancelled_count,
        },
        stage: task.stage,
        waiting_reason: task.waiting_reason,
        dependency_key: task.dependency_key,
        failure_stage: task.failure_stage,
        error_summary: task.error_summary,
        eta_seconds,
        created_at: task.created_at,
        started_at: task.started_at,
        finished_at: task.finished_at,
        updated_at: task.updated_at,
    }
}

fn parse_origin(value: &str) -> Result<TaskOrigin> {
    match value {
        "manual" => Ok(TaskOrigin::Manual),
        "rerun" => Ok(TaskOrigin::Rerun),
        other => Err(anyhow!("unsupported task origin {other}")),
    }
}

fn parse_status(value: &str) -> Result<TaskStatus> {
    match value {
        "queued" => Ok(TaskStatus::Queued),
        "running" => Ok(TaskStatus::Running),
        "waiting" => Ok(TaskStatus::Waiting),
        "succeeded" => Ok(TaskStatus::Succeeded),
        "failed" => Ok(TaskStatus::Failed),
        "cancelled" => Ok(TaskStatus::Cancelled),
        other => Err(anyhow!("unsupported task status {other}")),
    }
}

fn task_item_response(item: StoredTaskItemWithExternalJob) -> TaskItemResponse {
    TaskItemResponse {
        item_id: item.id,
        ordinal: item.ordinal,
        status: parse_item_status(&item.status).unwrap_or(TaskItemStatus::Failed),
        resource_id: item.resource_id,
        file_id: item.file_id,
        stage: item.stage,
        waiting_reason: item.waiting_reason,
        dependency_key: item.dependency_key,
        next_attempt_at: item.next_attempt_at,
        failure_stage: item.failure_stage,
        error_message: item.error_message,
        attempt_count: item.attempt_count,
        retryable: item.retryable,
        created_at: item.created_at,
        started_at: item.started_at,
        finished_at: item.finished_at,
        external_job: item.external_job_provider.map(|provider| ExternalJobInfo {
            provider,
            remote_task_id: item.external_job_remote_task_id.unwrap_or_default(),
            status: item.external_job_status.unwrap_or_default(),
            remote_status: item.external_job_remote_status,
            submitted_at: item.external_job_submitted_at.unwrap_or(item.created_at),
            last_polled_at: item.external_job_last_polled_at,
            next_poll_at: item.external_job_next_poll_at,
            deadline_at: item.external_job_deadline_at,
            error_message: item.external_job_error_message,
        }),
    }
}

fn parse_item_status(value: &str) -> Result<TaskItemStatus> {
    match value {
        "queued" => Ok(TaskItemStatus::Queued),
        "running" => Ok(TaskItemStatus::Running),
        "waiting" => Ok(TaskItemStatus::Waiting),
        "succeeded" => Ok(TaskItemStatus::Succeeded),
        "failed" => Ok(TaskItemStatus::Failed),
        "cancelled" => Ok(TaskItemStatus::Cancelled),
        other => Err(anyhow!("unsupported task item status {other}")),
    }
}

fn group_response(group: GroupRecord) -> GroupResponse {
    GroupResponse {
        group_id: group.id,
        group_key: group.group_key,
        group_path: Some(group.group_path),
        parent_group_path: group.parent_group_path,
        name: group.name,
        visibility: group.visibility,
        kind: group.kind,
        current_role: group.current_role,
        created_at: group.created_at,
        updated_at: group.updated_at,
    }
}

fn is_conflict_error(error: &anyhow::Error) -> bool {
    let message = error.to_string().to_ascii_lowercase();
    message.contains("duplicate key")
        || message.contains("unique constraint")
        || message.contains("already exists")
        || message.contains("conflict")
}

#[cfg(test)]
mod tests {
    use super::{parse_kind, split_scope_path};
    use context69_contracts::TaskKind;

    #[test]
    fn scope_path_is_split_without_empty_segments() {
        assert_eq!(
            split_scope_path("research/news").expect("scope"),
            (Some("research".into()), "news".into())
        );
        assert_eq!(
            split_scope_path("/news/").expect("scope"),
            (None, "news".into())
        );
        assert!(split_scope_path("/").is_err());
    }

    #[test]
    fn all_public_task_kinds_round_trip() {
        for kind in [
            TaskKind::SourceSync,
            TaskKind::TextBatch,
            TaskKind::FileBatch,
            TaskKind::UrlBatch,
            TaskKind::DeleteBatch,
            TaskKind::Translation,
            TaskKind::VectorRebuild,
        ] {
            assert_eq!(parse_kind(kind.as_str()).expect("kind"), kind);
        }
    }
}
