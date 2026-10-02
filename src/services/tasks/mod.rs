use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use anyhow::Result;
use context69_contracts::TaskKind;
use context69_translation::TranslationService;
use serde_json::Value;
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore};
use uuid::Uuid;

use crate::{
    db::{Database, StoredTask},
    domain_errors::DomainError,
    services::{
        document_store::DocumentStoreService, library::LibraryService, namespace::NamespaceService,
        settings::SettingsService, source_folders::SourceFoldersService, sync::SyncService,
    },
};

mod actions;
mod dispatcher;
mod docling_finalize;
mod docling_poll;
mod docling_submit;
pub mod events;
mod inline_waits;
mod item_file_processors;
mod item_git_index_processor;
mod item_lifecycle_processors;
#[cfg(test)]
mod item_pipeline_tests;
mod item_processors;
mod item_translation_processors;
mod item_url_processor;
mod lifecycle;
mod maintenance;
mod responses;
mod runtime;
mod scope;
#[cfg(test)]
mod sql_contract_tests;
mod submit;

pub use events::{TASK_EVENT_BUS_CAPACITY, TASK_EVENTS_CHANNEL, TaskEvent};

#[derive(Clone)]
pub struct TaskService {
    db: Database,
    namespace: NamespaceService,
    document_store: DocumentStoreService,
    library: LibraryService,
    sync: SyncService,
    source_folders: SourceFoldersService,
    translation: TranslationService,
    settings: SettingsService,
    worker_slots: Arc<Semaphore>,
    worker_capacity: usize,
    dispatch_notify: Arc<Notify>,
    dispatcher_started: Arc<AtomicBool>,
    task_event_bus: tokio::sync::broadcast::Sender<TaskEvent>,
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

pub(crate) fn normalize_task_worker_concurrency(concurrency: usize) -> usize {
    // Shared task worker pool is driven by scheduler.max_concurrency.
    // Preserve at least one worker for direct constructor inputs.
    concurrency.max(1)
}

/// Grouped dependencies for constructing a [`TaskService`].
#[derive(Clone)]
pub struct TaskServiceDependencies {
    pub db: Database,
    pub namespace: NamespaceService,
    pub document_store: DocumentStoreService,
    pub library: LibraryService,
    pub sync: SyncService,
    pub source_folders: SourceFoldersService,
    pub translation: TranslationService,
    pub concurrency: usize,
}

impl TaskService {
    pub fn new(dependencies: TaskServiceDependencies) -> Self {
        let TaskServiceDependencies {
            db,
            namespace,
            document_store,
            library,
            sync,
            source_folders,
            translation,
            concurrency,
        } = dependencies;
        let worker_capacity = normalize_task_worker_concurrency(concurrency);
        let (task_event_bus, _) = tokio::sync::broadcast::channel(
            crate::services::tasks::events::TASK_EVENT_BUS_CAPACITY,
        );
        Self {
            settings: SettingsService::new(db.clone()),
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
            task_event_bus,
        }
    }

    /// Subscribe to the process-local task event broadcast. Callers must
    /// full-sync once on subscribe, then apply incremental events
    /// (best-effort at-least-once fan-out from PG NOTIFY).
    pub fn subscribe_task_events(&self) -> tokio::sync::broadcast::Receiver<TaskEvent> {
        self.task_event_bus.subscribe()
    }

    /// Start the resident PG LISTEN -> broadcast hub. Every replica calls
    /// this at startup so cross-replica commits are visible locally.
    /// The hub reconnects transparently; failures only log and retry.
    pub fn start_event_bus(&self) {
        crate::services::tasks::events::spawn_task_event_listener_into(
            self.db.pool().clone(),
            self.task_event_bus.clone(),
        );
    }

    /// Test/shutdown-aware variant of [`Self::start_event_bus`].
    pub fn start_event_bus_with_shutdown(&self, shutdown: tokio_util::sync::CancellationToken) {
        crate::services::tasks::events::spawn_task_event_listener_into_with_shutdown(
            self.db.pool().clone(),
            self.task_event_bus.clone(),
            shutdown,
        );
    }

    pub fn resume_pending(&self) {
        dispatcher::start(self);
    }

    pub fn start_maintenance(&self) {
        maintenance::start(self);
    }

    /// Test/shutdown-aware variant: same dispatcher loop as
    /// [`Self::start_maintenance`] but exits when `shutdown` cancels.
    pub fn start_maintenance_with_shutdown(&self, shutdown: tokio_util::sync::CancellationToken) {
        maintenance::start_with_shutdown(self, shutdown);
    }

    /// Wakes the dispatcher after recovery adopts parked items so they resume
    /// without waiting for the 30s recovery tick.
    pub(crate) fn notify_dispatch(&self) {
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
            .ok_or_else(|| DomainError::not_found("task disappeared"))
            .map_err(anyhow::Error::from)
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
    pub(crate) fn settings(&self) -> &SettingsService {
        &self.settings
    }
    /// Uses the application's shared settings service (which carries the
    /// registered settings observers) instead of the per-service instance
    /// built in [`Self::new`].
    pub fn with_settings(mut self, settings: SettingsService) -> Self {
        self.settings = settings;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_task_worker_concurrency;

    #[test]
    fn task_worker_concurrency_clamps_zero_and_preserves_capacity() {
        assert_eq!(normalize_task_worker_concurrency(0), 1);
        assert_eq!(normalize_task_worker_concurrency(1), 1);
        assert_eq!(normalize_task_worker_concurrency(8), 8);
        assert_eq!(normalize_task_worker_concurrency(2), 2);
    }
}
