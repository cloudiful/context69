use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use anyhow::Result;
use context69_contracts::TaskKind;
use context69_translation::TranslationService;
use serde_json::Value;
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;
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
pub mod cutover;
mod dispatcher;
mod docling_cleanup;
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

/// The claim is a global primitive over the shared scratch database, so every
/// case that claims an item serializes on this lock.
#[cfg(test)]
pub(crate) static TASK_CASE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
mod item_processors;
mod item_translation_processors;
mod item_url_processor;
mod lifecycle;
mod lifecycle_logging;
mod maintenance;
mod pipeline_progress;
mod pipeline_progress_input;
mod responses;
mod runtime;
mod runtime_driver;
mod scope;
#[cfg(test)]
mod sql_contract_tests;
#[cfg(test)]
mod sql_contract_tests_p3;
mod submit;
mod task_diagnostics;

pub use events::{TASK_EVENT_BUS_CAPACITY, TASK_EVENTS_CHANNEL, TaskEvent};
pub(crate) use maintenance::MaintenanceHandles;

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
    dispatch_stop: Arc<DispatchStop>,
    task_event_bus: tokio::sync::broadcast::Sender<TaskEvent>,
    execution_latch: Arc<cutover::ExecutionLatch>,
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
            dispatch_stop: Arc::new(DispatchStop::new()),
            task_event_bus,
            execution_latch: cutover::ExecutionLatch::new(),
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

    /// Shutdown-aware event bus that returns the listener handle.
    pub(crate) fn start_event_bus_with_shutdown_owned(
        &self,
        shutdown: tokio_util::sync::CancellationToken,
    ) -> tokio::task::JoinHandle<()> {
        crate::services::tasks::events::spawn_task_event_listener_into_with_shutdown_owned(
            self.db.pool().clone(),
            self.task_event_bus.clone(),
            shutdown,
        )
    }

    pub fn resume_pending(&self) {
        dispatcher::start(self);
    }

    /// Idempotently stop admitting new work. Already-issued claims keep their
    /// owned workers so their leases and persisted progress stay truthful.
    pub fn begin_shutdown(&self) {
        self.dispatch_stop.begin();
        self.dispatch_notify.notify_waiters();
    }

    /// Wait until the dispatcher loop can no longer issue a claim.
    pub async fn wait_for_dispatch_stop(&self) {
        self.dispatch_stop.wait().await;
    }

    /// Wait until no item worker holds a slot. Callers must stop the dispatcher
    /// first so no new worker can take a slot while this waits.
    pub async fn wait_until_idle(&self) {
        wait_for_all_slots(&self.worker_slots, self.worker_capacity).await;
    }

    pub fn start_maintenance(&self) {
        maintenance::start(self);
    }

    /// Test/shutdown-aware variant: same dispatcher loop as
    /// [`Self::start_maintenance`] but exits when `shutdown` cancels.
    pub fn start_maintenance_with_shutdown(&self, shutdown: tokio_util::sync::CancellationToken) {
        maintenance::start_with_shutdown(self, shutdown);
    }

    /// Recovery-profile maintenance start with awaitable loop handles.
    pub(crate) fn start_maintenance_with_shutdown_owned(
        &self,
        shutdown: tokio_util::sync::CancellationToken,
    ) -> MaintenanceHandles {
        maintenance::start_with_shutdown_owned(self, shutdown)
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

    /// The readiness of the execution engine, decided once per process.
    pub async fn execution_readiness(&self) -> cutover::ExecutionReadiness {
        self.execution_latch.readiness(&self.db).await
    }

    /// The decided readiness, once the decision was taken.
    #[must_use]
    pub fn execution_readiness_decided(&self) -> Option<cutover::ExecutionReadiness> {
        self.execution_latch.decided()
    }

    pub(super) fn dispatcher_shutdown(&self) -> CancellationToken {
        self.dispatch_stop.token()
    }

    pub(super) fn store_dispatcher_handle(&self, handle: Option<dispatcher::DispatcherHandle>) {
        let Some(handle) = handle else {
            return;
        };
        // The real handle is owned by a waiter task; the stored handle below is
        // the join of that waiter, so a cancelled stop keeps the completion
        // owned and resumable instead of detaching the dispatcher.
        let waiter = tokio::spawn(async move { handle.wait().await });
        self.dispatch_stop.store(waiter);
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
            if let Err(error) = runtime_driver::run_item(&service, item).await {
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

/// The durable stop gate of one dispatcher.
///
/// `begin` disables new claims synchronously, before any await, and is
/// idempotent. The completion below stays owned and resumable across a
/// cancelled wait.
pub(crate) struct DispatchStop {
    shutdown: CancellationToken,
    completion: DispatcherCompletion,
}

impl DispatchStop {
    fn new() -> Self {
        Self {
            shutdown: CancellationToken::new(),
            completion: DispatcherCompletion::new(),
        }
    }

    /// Stops acquisition immediately; already-issued claims keep their workers.
    pub(crate) fn begin(&self) {
        self.shutdown.cancel();
    }

    fn token(&self) -> CancellationToken {
        self.shutdown.clone()
    }

    fn store(&self, handle: tokio::task::JoinHandle<()>) {
        self.completion.store(handle);
    }

    async fn wait(&self) {
        self.completion.wait().await;
    }
}

/// Owned, resumable completion of the dispatcher loop.
///
/// The join handle is awaited by mutable reference behind this slot and is only
/// released after its completion has been observed, so dropping a stop future
/// mid-wait never detaches the dispatcher or fakes its completion.
struct DispatcherCompletion {
    slot: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl DispatcherCompletion {
    fn new() -> Self {
        Self {
            slot: tokio::sync::Mutex::new(None),
        }
    }

    fn store(&self, handle: tokio::task::JoinHandle<()>) {
        // Startup stores the only handle before any stop can contend for it.
        let mut slot = self
            .slot
            .try_lock()
            .expect("the dispatcher completion is stored before any stop");
        *slot = Some(handle);
    }

    async fn wait(&self) {
        let mut slot = self.slot.lock().await;
        let Some(handle) = slot.as_mut() else {
            return;
        };
        let _ = (&mut *handle).await;
        *slot = None;
    }
}

async fn wait_for_all_slots(slots: &Arc<Semaphore>, capacity: usize) {
    let permits = u32::try_from(capacity).unwrap_or(u32::MAX);
    if let Ok(permits) = slots.clone().acquire_many_owned(permits).await {
        drop(permits);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use tokio::sync::Semaphore;

    use super::{
        DispatchStop, DispatcherCompletion, normalize_task_worker_concurrency, wait_for_all_slots,
    };

    #[test]
    fn task_worker_concurrency_clamps_zero_and_preserves_capacity() {
        assert_eq!(normalize_task_worker_concurrency(0), 1);
        assert_eq!(normalize_task_worker_concurrency(1), 1);
        assert_eq!(normalize_task_worker_concurrency(8), 8);
        assert_eq!(normalize_task_worker_concurrency(2), 2);
    }

    #[tokio::test]
    async fn interrupted_completion_wait_keeps_the_dispatcher_handle_owned() {
        let completion = Arc::new(DispatcherCompletion::new());
        let (release, released) = tokio::sync::oneshot::channel::<()>();
        let (finished, finished_rx) = tokio::sync::oneshot::channel::<()>();
        completion.store(tokio::spawn(async move {
            let _ = released.await;
            let _ = finished.send(());
        }));

        // First waiter is cancelled while the loop is still running.
        let held = tokio::time::timeout(Duration::from_millis(20), completion.wait()).await;
        assert!(held.is_err(), "the dispatcher loop is still running");
        assert!(
            completion
                .slot
                .try_lock()
                .expect("the cancelled wait released its guard")
                .is_some(),
            "the stored handle is still owned after a cancelled wait"
        );
        assert!(
            !finished_rx.is_terminated(),
            "the cancellation must not detach the loop"
        );

        // A second wait is still pending on the same owned handle.
        let second = tokio::time::timeout(Duration::from_millis(20), completion.wait()).await;
        assert!(second.is_err(), "the retained handle is still pending");

        // The same owner resumes the join after the loop finishes.
        release.send(()).expect("release the dispatcher loop");
        tokio::time::timeout(Duration::from_secs(1), completion.wait())
            .await
            .expect("the retained handle is joinable");
        finished_rx
            .await
            .expect("the dispatcher loop completed under its owner");
    }

    #[tokio::test]
    async fn stopping_disables_acquisition_before_a_held_join_finishes() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let stop = Arc::new(DispatchStop::new());
        let claims = Arc::new(AtomicUsize::new(0));
        let token = stop.token();
        let claim_counter = claims.clone();
        let (loop_done, loop_done_rx) = tokio::sync::oneshot::channel::<()>();
        stop.store(tokio::spawn(async move {
            loop {
                if token.is_cancelled() {
                    break;
                }
                claim_counter.fetch_add(1, Ordering::SeqCst);
                tokio::task::yield_now().await;
            }
            let _ = loop_done.send(());
        }));

        // The loop is acquiring before the stop entry.
        for _ in 0..64 {
            if claims.load(Ordering::SeqCst) > 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(claims.load(Ordering::SeqCst) > 0, "claims were issued");
        assert!(!stop.token().is_cancelled());

        // A held join that outlives the stop entry and the five-second budget
        // scale; acquisition must already be closed while it is held.
        let held_listener = tokio::spawn(std::future::pending::<()>());
        stop.begin();
        assert!(
            stop.token().is_cancelled(),
            "the stop entry disables acquisition synchronously"
        );
        let claims_at_stop = claims.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(
            claims.load(Ordering::SeqCst),
            claims_at_stop,
            "no claim is issued after the stop entry"
        );
        assert!(
            !held_listener.is_finished(),
            "the held join is still pending while acquisition is already closed"
        );

        // The owned completion of the loop is still joinable, not detached.
        tokio::time::timeout(Duration::from_secs(1), stop.wait())
            .await
            .expect("the owned loop completion is joinable");
        loop_done_rx.await.expect("the claim loop exited");
        held_listener.abort();
    }

    #[tokio::test]
    async fn idle_wait_returns_when_every_slot_is_free() {
        let slots = Arc::new(Semaphore::new(2));

        let started = Instant::now();
        wait_for_all_slots(&slots, 2).await;

        assert!(started.elapsed() < Duration::from_secs(1));
        assert_eq!(slots.available_permits(), 2);
    }

    #[tokio::test]
    async fn idle_wait_blocks_until_the_held_slot_is_released() {
        let slots = Arc::new(Semaphore::new(2));
        let held = slots
            .clone()
            .try_acquire_owned()
            .expect("second slot is free");
        let release = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            drop(held);
        });

        let started = Instant::now();
        wait_for_all_slots(&slots, 2).await;
        release.await.expect("releaser");

        assert!(started.elapsed() >= Duration::from_millis(50));
        assert_eq!(slots.available_permits(), 2);
    }
}
