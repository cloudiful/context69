use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use context69_contracts::{
    CreateGroupRequest, CreateMetadataIndexRequest, EnsureScopeResponse, GroupResponse,
    MetadataIndexResponse, MetadataIndexStatus, ScopeSpec, TaskItemResponse, TaskItemStatus,
    TaskItemsResponse, TaskKind, TaskListQuery, TaskPageResponse, TaskProgress, TaskRef,
    TaskResponse, TaskRetryResponse, TaskStatus,
};
use context69_translation::TranslationService;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::time::{interval, sleep};
use uuid::Uuid;

use crate::{
    db::{Database, StoredTask, StoredTaskItem},
    domain::GroupRecord,
    services::{
        document_store::DocumentStoreService, library::LibraryService, namespace::NamespaceService,
        sync::SyncService,
    },
};

mod runtime;

#[derive(Clone)]
pub struct TaskService {
    db: Database,
    namespace: NamespaceService,
    document_store: DocumentStoreService,
    library: LibraryService,
    sync: SyncService,
    translation: TranslationService,
}

#[derive(Debug, Clone)]
pub struct TaskSubmission {
    pub user_id: i64,
    pub group_id: Option<i64>,
    pub group_path: Option<String>,
    pub source_key: Option<String>,
    pub kind: TaskKind,
    pub payloads: Vec<Value>,
    pub idempotency_key: Option<String>,
}

impl TaskService {
    pub fn new(
        db: Database,
        namespace: NamespaceService,
        document_store: DocumentStoreService,
        library: LibraryService,
        sync: SyncService,
        translation: TranslationService,
    ) -> Self {
        Self {
            db,
            namespace,
            document_store,
            library,
            sync,
            translation,
        }
    }

    pub fn resume_pending(&self) {
        let service = self.clone();
        tokio::spawn(async move {
            let mut ticker = interval(Duration::from_secs(30));
            loop {
                ticker.tick().await;
                match service.db.pending_task_ids().await {
                    Ok(ids) => ids.into_iter().for_each(|id| service.spawn(id)),
                    Err(error) => tracing::warn!(%error, "failed to resume context69 tasks"),
                }
            }
        });
    }

    pub async fn submit(&self, request: TaskSubmission) -> Result<TaskRef> {
        if request.payloads.is_empty() {
            return Err(anyhow!("a task must contain at least one item"));
        }
        let request_hash = hash_payload(&request);
        let key = request
            .idempotency_key
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty());
        let task_id = Uuid::new_v4();
        let (task_id, reused) = self
            .db
            .create_task_submission(
                task_id,
                request.user_id,
                request.group_id,
                request.kind.as_str(),
                request.group_path.as_deref(),
                request.source_key.as_deref(),
                &request.payloads,
                key,
                &request_hash,
            )
            .await?;
        if !reused {
            self.spawn(task_id);
        }
        Ok(TaskRef { task_id })
    }

    pub async fn get(&self, task_id: Uuid, user_id: i64) -> Result<TaskResponse> {
        self.db
            .get_task(task_id, user_id)
            .await?
            .context("task not found")
            .map(task_response)
    }

    pub async fn list(&self, user_id: i64, query: &TaskListQuery) -> Result<TaskPageResponse> {
        if query.page == 0 || !(1..=100).contains(&query.page_size) {
            return Err(anyhow!(
                "page must be >= 1 and page_size must be between 1 and 100"
            ));
        }
        let kind = query.kind.map(TaskKind::as_str);
        let status = query.status.map(TaskStatus::as_str);
        let total = u64::try_from(self.db.count_tasks(user_id, kind, status).await?)?;
        let page_size = i64::from(query.page_size);
        let offset = i64::from(query.page - 1)
            .checked_mul(page_size)
            .context("task page offset is too large")?;
        let items = self
            .db
            .list_tasks(user_id, kind, status, page_size, offset)
            .await?
            .into_iter()
            .map(task_response)
            .collect();
        Ok(TaskPageResponse {
            items,
            page: query.page,
            page_size: query.page_size,
            total,
            total_pages: if total == 0 {
                0
            } else {
                total.div_ceil(u64::from(query.page_size)) as u32
            },
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
        let task = self
            .db
            .get_task(task_id, user_id)
            .await?
            .context("task not found")?;
        let all_items = self.list_all_task_items(task_id).await?;
        let failed = all_items
            .iter()
            .filter(|item| item.status == "failed" && item.retryable)
            .map(|item| item.ordinal)
            .collect::<std::collections::HashSet<_>>();
        let payloads = self
            .db
            .list_task_payloads(task_id)
            .await?
            .into_iter()
            .filter(|item| failed.contains(&item.ordinal))
            .map(|item| item.payload)
            .collect::<Vec<_>>();
        if payloads.is_empty() {
            return Err(anyhow!("task has no retryable failed items"));
        }
        let new_task = self
            .submit(TaskSubmission {
                user_id,
                group_id: task.group_id,
                group_path: task.group_path,
                source_key: task.source_key,
                kind: parse_kind(&task.kind)?,
                payloads,
                idempotency_key: None,
            })
            .await?;
        Ok(TaskRetryResponse {
            task: new_task,
            retried_items: failed.len() as i64,
        })
    }

    pub async fn cancel(&self, task_id: Uuid, user_id: i64) -> Result<()> {
        self.db
            .get_task(task_id, user_id)
            .await?
            .context("task not found")?;
        if self.db.cancel_task(task_id, user_id).await? {
            Ok(())
        } else {
            Err(anyhow!("task is already terminal or not found"))
        }
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

    fn spawn(&self, task_id: Uuid) {
        let service = self.clone();
        tokio::spawn(async move {
            if let Err(error) = service.run(task_id).await {
                tracing::warn!(task_id = %task_id, %error, "context69 task worker failed");
            }
        });
    }

    async fn run(&self, task_id: Uuid) -> Result<()> {
        let lease = Uuid::new_v4();
        if !self.db.claim_task(task_id, lease).await? {
            return Ok(());
        }
        if let Err(error) = runtime::run_task(self, task_id, lease).await {
            self.db
                .fail_task(task_id, lease, "worker", &error.to_string())
                .await?;
            return Err(error);
        }
        Ok(())
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
    pub(crate) fn namespace(&self) -> &NamespaceService {
        &self.namespace
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
    pub(crate) fn translation(&self) -> &TranslationService {
        &self.translation
    }

    async fn list_all_task_items(&self, task_id: Uuid) -> Result<Vec<StoredTaskItem>> {
        const PAGE_SIZE: i64 = 500;
        let mut offset = 0;
        let mut all = Vec::new();
        loop {
            let page = self.db.list_task_items(task_id, PAGE_SIZE, offset).await?;
            let page_len = page.len() as i64;
            all.extend(page);
            if page_len < PAGE_SIZE {
                return Ok(all);
            }
            offset += page_len;
        }
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
        group_path: task.group_path,
        source_key: task.source_key,
        progress: TaskProgress {
            total: task.total_count,
            queued: task.queued_count,
            running: task.running_count,
            succeeded: task.succeeded_count,
            failed: task.failed_count,
            cancelled: task.cancelled_count,
        },
        failure_stage: task.failure_stage,
        error_summary: task.error_summary,
        eta_seconds,
        created_at: task.created_at,
        started_at: task.started_at,
        finished_at: task.finished_at,
        updated_at: task.updated_at,
    }
}

fn parse_status(value: &str) -> Result<TaskStatus> {
    match value {
        "queued" => Ok(TaskStatus::Queued),
        "running" => Ok(TaskStatus::Running),
        "succeeded" => Ok(TaskStatus::Succeeded),
        "failed" => Ok(TaskStatus::Failed),
        "cancelled" => Ok(TaskStatus::Cancelled),
        other => Err(anyhow!("unsupported task status {other}")),
    }
}

fn task_item_response(item: StoredTaskItem) -> TaskItemResponse {
    TaskItemResponse {
        item_id: item.id,
        ordinal: item.ordinal,
        status: parse_item_status(&item.status).unwrap_or(TaskItemStatus::Failed),
        resource_id: item.resource_id,
        failure_stage: item.failure_stage,
        error_message: item.error_message,
        attempt_count: item.attempt_count,
        retryable: item.retryable,
        created_at: item.created_at,
        started_at: item.started_at,
        finished_at: item.finished_at,
    }
}

fn parse_item_status(value: &str) -> Result<TaskItemStatus> {
    match value {
        "queued" => Ok(TaskItemStatus::Queued),
        "running" => Ok(TaskItemStatus::Running),
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
