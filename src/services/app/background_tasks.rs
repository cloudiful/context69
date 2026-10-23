use std::sync::Arc;

use anyhow::Result;
use tracing::warn;

use crate::{
    config::Config,
    db::Database,
    services::{
        document_store::DocumentStoreService,
        library::LibraryService,
        metadata_payload_sync::MetadataPayloadSyncWorker,
        namespace::NamespaceService,
        settings::{ConfigRefreshOutcome, SettingsService},
        tasks::{TaskService, TaskServiceDependencies},
    },
};

use super::{
    library_startup::LibraryStartup,
    services_init::ServicesInit,
    task_worker_capacity,
    vector_runtime::{
        EmbeddingRuntimeReloader, EmbeddingRuntimeReloaderContext, EmbeddingSettingsGuard,
        EmbeddingTransitionApplier, VectorRuntime,
    },
};
use crate::services::settings::RuntimeEmbeddingTransition;

pub struct BackgroundTasks {
    pub document_store: DocumentStoreService,
    pub tasks: TaskService,
}

pub async fn start(
    db: &Database,
    config: &Config,
    namespace: &NamespaceService,
    settings: &mut SettingsService,
    vector: &VectorRuntime,
    services: &ServicesInit,
    startup: &LibraryStartup,
) -> Result<BackgroundTasks> {
    settings.set_docling_settings_observer(Some(Arc::new({
        let library = startup.library.clone();
        move || {
            let library = library.clone();
            tokio::spawn(async move {
                if let Err(error) = library.refresh_dependency_configuration().await {
                    warn!(
                        %error,
                        "failed to refresh dependency gates after docling settings change"
                    );
                }
            });
        }
    })));
    // While a fixed vector index is live, only credential and timeout changes
    // are hot-swappable: a model/base-URL/dimensions change is rejected here,
    // before any secret or settings write, so the save fails loudly instead of
    // being accepted and silently ignored. An identity-changing save is also
    // refused while a live reload or rebuild is in progress, so a racing save
    // can never report success while its identity remains unapplied.
    let embedding_reload_in_progress = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let embedding_reconciliation_required = Arc::new(std::sync::atomic::AtomicBool::new(false));
    settings.set_runtime_embedding_guard(Some(Arc::new({
        let guard = EmbeddingSettingsGuard::new(
            vector.embedding.clone(),
            vector.vector.clone(),
            embedding_reload_in_progress.clone(),
        )
        .with_reconciliation_required(embedding_reconciliation_required.clone());
        move |embedding| guard.check(embedding)
    })));
    // One exclusion shared by an identity transition, a live reload, and a
    // manual rebuild start, so none of them can interleave their checks.
    settings.set_runtime_embedding_transition(Some(RuntimeEmbeddingTransition::new(
        {
            let vector = vector.vector.clone();
            move || vector.try_begin_identity_transition()
        },
        {
            let vector = vector.vector.clone();
            move || vector.end_identity_transition()
        },
    )));
    // The live runtime is the only source for a populated collection that has
    // no recorded identity, so the response consults it alongside the row.
    settings.set_runtime_requires_rebuild(Some({
        let vector = vector.vector.clone();
        Arc::new(move || vector.requires_rebuild())
    }));
    // A destination change would make a running rebuild's completion describe a
    // different collection, even with an unchanged embedding identity.
    settings.set_runtime_rebuild_in_progress(Some({
        let vector = vector.vector.clone();
        Arc::new(move || vector.is_rebuilding())
    }));
    // A confirmed same-space transition applies only after its settings row is
    // committed, and the save reports success only when this returns.
    settings.set_runtime_embedding_applier(Some({
        let applier = Arc::new(EmbeddingTransitionApplier::new(
            vector.embedding.clone(),
            vector.vector.clone(),
            vector.recovery.clone(),
            config.clone(),
            embedding_reconciliation_required,
        ));
        Arc::new(move |provider, identity, stored, api_key| {
            let applier = applier.clone();
            Box::pin(async move { applier.apply(provider, identity, stored, api_key).await })
        })
    }));
    // A runtime settings save can change the S3 configuration, so refresh the
    // gates and re-probe the S3 backend through the same observer/probe path as
    // the periodic recovery loop. The probe never mutates stored objects. The
    // same save can change the embedding credential or timeout, so the reloader
    // installs that (identity-preserving) change on the shared runtime.
    let embedding_reloader = Arc::new(EmbeddingRuntimeReloader::new(
        EmbeddingRuntimeReloaderContext {
            runtime: vector.embedding.clone(),
            vector: vector.vector.clone(),
            base_config: config.clone(),
            reload_in_progress: embedding_reload_in_progress.clone(),
        },
    ));
    // One authoritative refresh path: a settings observer only requests a
    // refresh, it never hands over a configuration it read earlier. The refresh
    // re-reads the committed row under the settings save lock and the shared
    // runtime transition, so a delayed observer can never install an older
    // configuration over a newer save or acknowledgement, and a deferred retry
    // always reloads the newest state instead of replaying a captured one.
    settings.set_runtime_config_refresher(Some(Arc::new({
        let db = db.clone();
        let store = services.secrets.clone();
        let base_config = config.clone();
        let embedding_reloader = embedding_reloader.clone();
        let recovery = Arc::clone(&vector.recovery);
        move || {
            let db = db.clone();
            let store = store.clone();
            let base_config = base_config.clone();
            let embedding_reloader = embedding_reloader.clone();
            let recovery = recovery.clone();
            Box::pin(async move {
                let Some(stored) =
                    super::runtime_settings::load_runtime_settings_readonly(&db, &store).await?
                else {
                    return Ok(ConfigRefreshOutcome::AppliedUnchanged);
                };
                embedding_reloader.reload_reserved(&stored).await?;
                let mut effective = base_config;
                super::runtime_settings::apply_runtime_settings(&mut effective, &stored);
                Ok(recovery.apply_reserved_reconfiguration(effective))
            })
        }
    })));
    let refresh_service = settings.clone();
    let refresh_signal = Arc::new(tokio::sync::Notify::new());
    // The same save can change the S3 configuration, so refresh the gates and
    // re-probe the S3 backend through the same observer/probe path as the
    // periodic recovery loop. The probe never mutates stored objects.
    settings.set_runtime_settings_observer(Some(Arc::new({
        let library = startup.library.clone();
        let refresh_service = refresh_service.clone();
        let refresh_signal = refresh_signal.clone();
        move || {
            let library = library.clone();
            refresh_service.request_runtime_refresh();
            refresh_signal.notify_one();
            tokio::spawn(async move {
                if let Err(error) = library.refresh_dependency_configuration().await {
                    warn!(
                        %error,
                        "failed to refresh dependency gates after runtime settings change"
                    );
                }
                if let Err(error) = library.probe_s3_gate().await {
                    warn!(
                        %error,
                        "failed to probe the s3 dependency gate after runtime settings change"
                    );
                }
            });
        }
    })));
    {
        let shutdown = vector.recovery.shutdown_token();
        tokio::spawn(async move {
            refresh_service
                .run_runtime_refresh_loop(refresh_signal, shutdown)
                .await;
        });
    }
    // Bounded S3 gate recovery (issue 702 P2): reserve the half-open probe lease,
    // run one read-only backend check, and record the outcome. A no-op when S3
    // is not the active backend.
    {
        let library = startup.library.clone();
        let shutdown = tokio_util::sync::CancellationToken::new();
        tokio::spawn(async move {
            library.run_s3_gate_recovery(shutdown).await;
        });
    }
    let document_store = DocumentStoreService::with_vector_runtime(
        db.clone(),
        vector.vector.clone(),
        startup.library.clone(),
    );
    document_store.resume_pending();
    vector
        .recovery
        .spawn(Arc::new(super::vector_recovery::AppRecoveryObserver::new(
            vector.embedding.clone(),
            startup.library.clone(),
            document_store.clone(),
        )));
    // Durable convergence of the retired enrichment payload mirror. The worker
    // reads the shared runtime, waits while no pair is ready (including an
    // initially unconfigured instance), and shuts down with the recovery
    // controller that owns this process's vector lifetime.
    MetadataPayloadSyncWorker::new(db.clone(), vector.vector.clone())
        .spawn(vector.recovery.shutdown_token());
    let tasks = TaskService::new(TaskServiceDependencies {
        db: db.clone(),
        namespace: namespace.clone(),
        document_store: document_store.clone(),
        library: startup.library.clone(),
        sync: services.sync.clone(),
        source_folders: startup.source_folders.clone(),
        translation: services.translation.clone(),
        concurrency: task_worker_capacity(config),
    })
    .with_settings(settings.clone());
    tasks.resume_pending();
    tasks.start_maintenance();
    tasks.start_event_bus();
    services.translation.resume().await?;
    if let Err(error) = db.delete_expired_rerank_item_scores(30).await {
        warn!(error = %error, "failed to prune expired rerank item scores during startup");
    }
    if services.startup_requires_rebuild {
        // Reported, never repaired implicitly: startup performs zero embedding
        // calls and never deletes or recreates the collection. Only an explicit
        // manual rebuild (or a confirmed same-space acknowledgment) may clear
        // this state.
        warn!(
            collection_name = config.qdrant.collection_name,
            "persisted vectors do not match the configured embedding identity; vector work stays \
             unavailable until an explicit rebuild completes"
        );
    }

    Ok(BackgroundTasks {
        document_store,
        tasks,
    })
}

/// Start the upgrade-recovery profile.
///
/// Only writers that finish already persisted task work are started; normal
/// startup sweeps, the metadata-index worker, the payload-mirror worker, the
/// scheduler, and MCP stay off. Every started loop is owned and joinable so the
/// caller can prove local writer quiescence before the migration runs.
/// The already-built handles the recovery profile wires into its task service.
pub struct UpgradeRecoveryServices<'a> {
    pub namespace: &'a NamespaceService,
    pub settings: &'a SettingsService,
    pub sync: &'a crate::services::sync::SyncService,
    pub translation: &'a context69_translation::TranslationService,
    pub startup: &'a LibraryStartup,
}

pub async fn start_upgrade_recovery(
    db: &Database,
    config: &Config,
    vector: &VectorRuntime,
    services: UpgradeRecoveryServices<'_>,
) -> BackgroundTasks {
    let startup = services.startup;
    let document_store = DocumentStoreService::with_vector_runtime(
        db.clone(),
        vector.vector.clone(),
        startup.library.clone(),
    );
    let tasks = TaskService::new(TaskServiceDependencies {
        db: db.clone(),
        namespace: services.namespace.clone(),
        document_store: document_store.clone(),
        library: startup.library.clone(),
        sync: services.sync.clone(),
        source_folders: startup.source_folders.clone(),
        translation: services.translation.clone(),
        concurrency: task_worker_capacity(config),
    })
    .with_settings(services.settings.clone());
    BackgroundTasks {
        document_store,
        tasks,
    }
}

/// Every background loop the upgrade-recovery profile owns.
pub struct UpgradeRecoveryWriters {
    cancellation: tokio_util::sync::CancellationToken,
    vector_shutdown: tokio_util::sync::CancellationToken,
    s3_gate: Option<tokio::task::JoinHandle<()>>,
    vector_supervisor: Option<tokio::task::JoinHandle<()>>,
    maintenance: crate::services::tasks::MaintenanceHandles,
    event_bus: Option<tokio::task::JoinHandle<()>>,
}

/// Start the recovery-profile writers and return their owned handles.
pub fn start_upgrade_recovery_writers(
    tasks: &TaskService,
    library: &LibraryService,
    vector: &VectorRuntime,
    document_store: &DocumentStoreService,
) -> UpgradeRecoveryWriters {
    let cancellation = tokio_util::sync::CancellationToken::new();
    let s3_shutdown = cancellation.clone();
    let s3_library = library.clone();
    let s3_gate = tokio::spawn(async move {
        s3_library.run_s3_gate_recovery(s3_shutdown).await;
    });
    let observer = std::sync::Arc::new(
        super::vector_recovery::AppRecoveryObserver::without_document_store_resume(
            vector.embedding.clone(),
            library.clone(),
            document_store.clone(),
        ),
    );
    let vector_shutdown = vector.recovery.shutdown_token();
    let vector_supervisor = vector.recovery.spawn_owned(observer);
    tasks.resume_pending();
    let maintenance = tasks.start_maintenance_with_shutdown_owned(cancellation.clone());
    let event_bus = tasks.start_event_bus_with_shutdown_owned(cancellation.clone());
    UpgradeRecoveryWriters {
        cancellation,
        vector_shutdown,
        s3_gate: Some(s3_gate),
        vector_supervisor: Some(vector_supervisor),
        maintenance,
        event_bus: Some(event_bus),
    }
}

impl UpgradeRecoveryWriters {
    /// Stop acquisition, cancel every owned loop, and await the dispatcher,
    /// current item workers, and each loop handle.
    ///
    /// Cancellation-safe: every handle is awaited by mutable reference and is
    /// only released once it is observed to finish, so a stop interrupted by a
    /// signal resumes the same join under the remaining budget instead of
    /// detaching a loop or reporting a false completion.
    pub async fn stop_and_join(&mut self, tasks: &TaskService) {
        tasks.begin_shutdown();
        tasks.wait_for_dispatch_stop().await;
        tasks.wait_until_idle().await;
        self.cancellation.cancel();
        self.vector_shutdown.cancel();
        self.maintenance.wait().await;
        join_owned(&mut self.s3_gate).await;
        join_owned(&mut self.event_bus).await;
        join_owned(&mut self.vector_supervisor).await;
    }

    /// Cancel every owned loop without waiting for item workers.
    pub fn cancel(&self) {
        self.cancellation.cancel();
        self.vector_shutdown.cancel();
    }
}

async fn join_owned(slot: &mut Option<tokio::task::JoinHandle<()>>) {
    if let Some(task) = slot.as_mut() {
        let _ = (&mut *task).await;
        *slot = None;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use std::time::Duration;

    use super::join_owned;

    #[tokio::test]
    async fn an_interrupted_owned_join_keeps_the_handle_for_the_next_stop() {
        let (release, released) = tokio::sync::oneshot::channel::<()>();
        let done = Arc::new(AtomicBool::new(false));
        let task_done = done.clone();
        let mut slot = Some(tokio::spawn(async move {
            let _ = released.await;
            task_done.store(true, Ordering::SeqCst);
        }));

        let held = tokio::time::timeout(Duration::from_millis(20), join_owned(&mut slot)).await;
        assert!(held.is_err(), "the owned loop is still running");
        assert!(
            slot.is_some(),
            "a cancelled join keeps the handle owned instead of detaching it"
        );
        assert!(!done.load(Ordering::SeqCst));

        release.send(()).expect("release the owned loop");
        join_owned(&mut slot).await;
        assert!(slot.is_none(), "the handle is released after completion");
        assert!(done.load(Ordering::SeqCst), "the real loop finished");
    }
}
