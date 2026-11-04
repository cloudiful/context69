use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use anyhow::Result;
use context69_contracts::TaskKind;
use context69_translation::TranslationService;
use serde_json::Value;
use tokio::sync::{Notify, watch};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    db::{Database, RegistryQuotaOverride, StoredTask},
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
pub mod handover;
mod inline_waits;
mod item_file_processors;
mod item_git_index_processor;
mod item_lifecycle_processors;
#[cfg(test)]
mod item_pipeline_tests;
mod item_replacement_processor;

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
    /// Number of locally spawned item workers, used only by shutdown and
    /// `wait_until_idle`. It is liveness bookkeeping, never admission: the
    /// library queue policy owns execution capacity.
    in_flight: Arc<watch::Sender<usize>>,
    dispatch_notify: Arc<Notify>,
    dispatcher_started: Arc<AtomicBool>,
    dispatch_stop: Arc<DispatchStop>,
    task_event_bus: tokio::sync::broadcast::Sender<TaskEvent>,
    execution_latch: Arc<cutover::ExecutionLatch>,
    /// The registry-visible replacement processor of this service, or `None`
    /// while it stays disabled.
    ///
    /// The value is a narrowing of the persisted capacity policy, never the
    /// policy itself: the admission reads the persisted caps inside its own
    /// transaction, so this service never decides how much the registry may
    /// hold.
    replacement_caps: Option<RegistryQuotaOverride>,
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

/// Grouped dependencies for constructing a [`TaskService`].
///
/// There is no local worker-capacity input: execution capacity is the durable
/// library queue policy, applied at startup and on a runtime settings save.
#[derive(Clone)]
pub struct TaskServiceDependencies {
    pub db: Database,
    pub namespace: NamespaceService,
    pub document_store: DocumentStoreService,
    pub library: LibraryService,
    pub sync: SyncService,
    pub source_folders: SourceFoldersService,
    pub translation: TranslationService,
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
        } = dependencies;
        let (task_event_bus, _) = tokio::sync::broadcast::channel(
            crate::services::tasks::events::TASK_EVENT_BUS_CAPACITY,
        );
        // The atomic writers of this service follow the library service they
        // share the registry with: the persisted capacity policy decides
        // whether they run, and this service inherits that decision instead
        // of carrying a second one.
        let replacement_caps = library.registry_caps();
        Self {
            settings: SettingsService::new(db.clone()),
            db,
            namespace,
            document_store,
            library,
            sync,
            source_folders,
            translation,
            in_flight: Arc::new(watch::channel(0).0),
            dispatch_notify: Arc::new(Notify::new()),
            dispatcher_started: Arc::new(AtomicBool::new(false)),
            dispatch_stop: Arc::new(DispatchStop::new()),
            task_event_bus,
            execution_latch: cutover::ExecutionLatch::new(),
            replacement_caps,
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

    /// Wait until no item worker spawned by this service is running. Callers
    /// must stop the dispatcher first so no new worker can start while this
    /// waits. The count is local liveness, not an admission limit.
    pub async fn wait_until_idle(&self) {
        let mut in_flight = self.in_flight.subscribe();
        loop {
            let running = *in_flight.borrow_and_update();
            if running == 0 {
                break;
            }
            if in_flight.changed().await.is_err() {
                break;
            }
        }
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

    pub(super) fn spawn_item(&self, item: crate::db::ClaimedItem) {
        let service = self.clone();
        service.in_flight.send_modify(|count| *count += 1);
        tokio::spawn(async move {
            if let Err(error) = runtime_driver::run_item(&service, item).await {
                tracing::warn!(%error, "context69 task item worker failed");
            }
            service
                .in_flight
                .send_modify(|count| *count = count.saturating_sub(1));
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
    /// The registry-visible replacement processor, or `None` while it stays
    /// disabled (issue 1138 P17).
    pub(super) fn replacement_caps(&self) -> Option<RegistryQuotaOverride> {
        self.replacement_caps
    }

    /// Test-only setup of the replacement caps.
    ///
    /// The caps only narrow the persisted policy: a test can lower the
    /// registry capacity, never raise it.
    #[cfg(any(test, feature = "integration-test-helpers"))]
    #[must_use]
    pub fn with_replacement_caps_for_test(mut self, caps: [i64; 3]) -> Self {
        self.replacement_caps = Some(RegistryQuotaOverride::exact(caps));
        self
    }

    /// Test-only drive of one claimed item through the production worker path.
    #[cfg(feature = "integration-test-helpers")]
    pub async fn run_item_for_test(&self, item: crate::db::ClaimedItem) -> Result<()> {
        runtime_driver::run_item(self, item).await
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use super::{DispatchStop, DispatcherCompletion};

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
}
