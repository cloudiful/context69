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
    library_startup::LibraryStartup, services_init::ServicesInit, task_worker_capacity,
    vector_rebuild, vector_runtime::VectorRuntime,
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
            services.vector_index_ready.clone(),
        );
    }

    Ok(BackgroundTasks {
        document_store,
        tasks,
    })
}
