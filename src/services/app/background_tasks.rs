use std::sync::Arc;

use anyhow::Result;
use tracing::warn;

use crate::{
    config::Config,
    db::Database,
    services::{
        document_store::DocumentStoreService,
        metadata_payload_sync::MetadataPayloadSyncWorker,
        namespace::NamespaceService,
        settings::SettingsService,
        tasks::{TaskService, TaskServiceDependencies},
    },
};

use super::{
    library_startup::LibraryStartup,
    services_init::ServicesInit,
    task_worker_capacity, vector_rebuild,
    vector_runtime::{
        EmbeddingRuntimeReloader, EmbeddingRuntimeReloaderContext, EmbeddingSettingsGuard,
        VectorRuntime,
    },
};

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
    settings.set_runtime_embedding_guard(Some(Arc::new({
        let guard = EmbeddingSettingsGuard::new(
            vector.embedding.clone(),
            vector.vector.clone(),
            embedding_reload_in_progress.clone(),
        );
        move |embedding| guard.check(embedding)
    })));
    // Refreshes the dependency gates once a rebuild reopens reads, so the
    // embedding/qdrant gates closed during the rebuild become available again.
    let readiness_refresh: Arc<dyn Fn() + Send + Sync> = Arc::new({
        let library = startup.library.clone();
        move || {
            let library = library.clone();
            tokio::spawn(async move {
                if let Err(error) = library.refresh_dependency_configuration().await {
                    warn!(
                        %error,
                        "failed to refresh dependency gates after the vector index rebuild"
                    );
                }
            });
        }
    });
    // A runtime settings save can change the S3 configuration, so refresh the
    // gates and re-probe the S3 backend through the same observer/probe path as
    // the periodic recovery loop. The probe never mutates stored objects. The
    // same save can change the embedding credential or timeout, so the reloader
    // installs that (identity-preserving) change on the shared runtime.
    let embedding_reloader = Arc::new(EmbeddingRuntimeReloader::new(
        EmbeddingRuntimeReloaderContext {
            db: db.clone(),
            store: services.secrets.clone(),
            runtime: vector.embedding.clone(),
            vector: vector.vector.clone(),
            base_config: config.clone(),
            reload_in_progress: embedding_reload_in_progress.clone(),
        },
    ));
    settings.set_runtime_settings_observer(Some(Arc::new({
        let library = startup.library.clone();
        let embedding_reloader = embedding_reloader.clone();
        let settings_db = db.clone();
        let settings_store = services.secrets.clone();
        let base_config = config.clone();
        let recovery = Arc::clone(&vector.recovery);
        move || {
            let library = library.clone();
            let embedding_reloader = embedding_reloader.clone();
            let settings_db = settings_db.clone();
            let settings_store = settings_store.clone();
            let base_config = base_config.clone();
            let recovery = recovery.clone();
            tokio::spawn(async move {
                if let Err(error) = embedding_reloader.reload().await {
                    warn!(
                        %error,
                        "failed to reload the embedding runtime after runtime settings change"
                    );
                }
                // A vector-relevant setting change starts a new generation, so
                // an in-flight recovery attempt cannot publish the old pair and
                // the supervisor reconnects with the committed configuration.
                let mut effective = base_config;
                match super::runtime_settings::load_runtime_settings(&settings_db, &settings_store)
                    .await
                {
                    Ok(Some(stored)) => {
                        super::runtime_settings::apply_runtime_settings(&mut effective, &stored);
                        recovery.reconfigure(effective);
                    }
                    Ok(None) => {}
                    Err(error) => warn!(
                        %error,
                        "failed to load runtime settings for vector runtime reconfiguration"
                    ),
                }
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
    if services.automatic_rebuild_needed
        && !spawn_vector_rebuild(vector, &services.sync, db, config, &readiness_refresh)
    {
        // The pair is not published yet. Run the deferred initial rebuild
        // exactly once when recovery publishes a safe client; a cancellation
        // ends the wait.
        let vector_handle = vector.vector.clone();
        let gate = vector.gate.clone();
        let sync = services.sync.clone();
        let db = db.clone();
        let config = config.clone();
        let fingerprint = vector.fingerprint.clone();
        let recreate = vector.fingerprint_changed && !vector.collection_needs_rebuild;
        let readiness_refresh = readiness_refresh.clone();
        let shutdown = vector.recovery.shutdown_token();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown.cancelled() => return,
                    _ = tokio::time::sleep(std::time::Duration::from_millis(500)) => {}
                }
                let Some(snapshot) = vector_handle.snapshot() else {
                    continue;
                };
                let generation = vector_handle.begin_rebuild();
                vector_rebuild::spawn(
                    sync.clone(),
                    db.clone(),
                    vector_rebuild::RebuildTarget {
                        runtime: vector_handle.clone(),
                        index: snapshot.index,
                        generation,
                    },
                    config.clone(),
                    fingerprint.clone(),
                    recreate,
                    gate.rebuild_ticket()
                        .with_on_settled(readiness_refresh.clone()),
                );
                return;
            }
        });
    }

    Ok(BackgroundTasks {
        document_store,
        tasks,
    })
}

/// Starts the automatic rebuild when a vector pair is already published.
fn spawn_vector_rebuild(
    vector: &VectorRuntime,
    sync: &crate::services::sync::SyncService,
    db: &Database,
    config: &Config,
    readiness_refresh: &Arc<dyn Fn() + Send + Sync>,
) -> bool {
    let Some(snapshot) = vector.vector.snapshot() else {
        return false;
    };
    let generation = vector.vector.begin_rebuild();
    vector_rebuild::spawn(
        sync.clone(),
        db.clone(),
        vector_rebuild::RebuildTarget {
            runtime: vector.vector.clone(),
            index: snapshot.index,
            generation,
        },
        config.clone(),
        vector.fingerprint.clone(),
        vector.fingerprint_changed && !vector.collection_needs_rebuild,
        vector
            .gate
            .rebuild_ticket()
            .with_on_settled(readiness_refresh.clone()),
    );
    true
}
