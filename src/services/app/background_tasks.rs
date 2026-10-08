use std::sync::Arc;

use anyhow::Result;
use tracing::warn;

use crate::{
    config::Config,
    db::Database,
    services::{
        document_store::DocumentStoreService,
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
            vector.index.is_some(),
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
            index_present: vector.index.is_some(),
            base_config: config.clone(),
            reload_in_progress: embedding_reload_in_progress.clone(),
        },
    ));
    settings.set_runtime_settings_observer(Some(Arc::new({
        let library = startup.library.clone();
        let embedding_reloader = embedding_reloader.clone();
        move || {
            let library = library.clone();
            let embedding_reloader = embedding_reloader.clone();
            tokio::spawn(async move {
                if let Err(error) = embedding_reloader.reload().await {
                    warn!(
                        %error,
                        "failed to reload the embedding runtime after runtime settings change"
                    );
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
    let document_store =
        DocumentStoreService::new(db.clone(), vector.index.clone(), startup.library.clone());
    document_store.resume_pending();
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
    services.extraction.resume().await?;
    if let Err(error) = db.delete_expired_rerank_item_scores(30).await {
        warn!(error = %error, "failed to prune expired rerank item scores during startup");
    }
    if services.automatic_rebuild_needed {
        vector_rebuild::spawn(
            services.sync.clone(),
            db.clone(),
            vector
                .index
                .clone()
                .expect("automatic vector rebuild requires a qdrant index"),
            config.clone(),
            vector.fingerprint.clone(),
            vector.fingerprint_changed && !vector.collection_needs_rebuild,
            vector
                .gate
                .rebuild_ticket()
                .with_on_settled(readiness_refresh.clone()),
        );
    }

    Ok(BackgroundTasks {
        document_store,
        tasks,
    })
}
