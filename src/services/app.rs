use std::{
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

use anyhow::Result;
use context69_extraction::{ExtractionDependencies, ExtractionService};
use context69_translation::{TranslationDependencies, TranslationService};
use tracing::{info, warn};

use crate::{
    chunking::ChunkingConfig,
    config::{Config, ConnectionConfig},
    db::Database,
    embedding::{EmbeddingProvider, OpenAiCompatibleEmbeddingProvider},
    library_store::LibraryStore,
    qdrant_index::QdrantIndex,
    services::{
        auth::AuthService,
        document_store::DocumentStoreService,
        extraction::ExtractionPublisherAdapter,
        library::{DEFAULT_LEGACY_CLEANUP_BATCH_SIZE, LibraryService, MissingSourceCleanupSummary},
        namespace::NamespaceService,
        personal_access_tokens::PersonalAccessTokenService,
        query::QueryService,
        settings::SettingsService,
        source_folders::SourceFoldersService,
        sync::SyncService,
        tasks::TaskService,
        translation::TranslationPublisherAdapter,
    },
};

mod browser_sessions;
mod readiness;
mod runtime_settings;
mod vector_identity;
mod vector_rebuild;

pub use browser_sessions::BrowserSessionConfig;

#[derive(Clone)]
pub struct Context69App {
    pub config: Config,
    pub db: Database,
    pub auth: AuthService,
    pub personal_access_tokens: PersonalAccessTokenService,
    pub namespace: NamespaceService,
    pub query: QueryService,
    pub sync: SyncService,
    pub settings: SettingsService,
    pub library: LibraryService,
    pub source_folders: SourceFoldersService,
    pub document_store: DocumentStoreService,
    pub translation: TranslationService,
    pub extraction: ExtractionService,
    pub tasks: TaskService,
    pub browser_sessions: BrowserSessionConfig,
}

impl Context69App {
    pub async fn new(mut config: Config) -> Result<Self> {
        let db = Database::connect(&config.app_db.url).await?;
        let namespace = NamespaceService::new(db.clone());
        let auth = AuthService::new(db.clone(), config.auth.clone())?;
        let personal_access_tokens = PersonalAccessTokenService::new(db.clone(), auth.clone());
        auth.ensure_bootstrap_admin().await?;
        runtime_settings::import_legacy_runtime_if_needed(&db, &config).await?;

        let mut settings = SettingsService::new(db.clone());
        let runtime = runtime_settings::load_runtime_settings(&db).await?;
        if let Some(runtime) = &runtime {
            runtime_settings::apply_runtime_settings(&mut config, runtime);
        }
        let browser_sessions = browser_sessions::resolve(&db, &config).await?;
        config.connections = db
            .list_source_connections()
            .await?
            .into_iter()
            .map(|connection| ConnectionConfig {
                name: connection.name,
                database_url: connection.database_url,
            })
            .collect();
        config.docling = match settings.resolve_docling_config().await {
            Ok(docling) => docling,
            Err(error) => {
                warn!(error = %error, "docling settings are invalid; continuing without docling runtime");
                None
            }
        };

        let mut embedding: Option<Arc<dyn EmbeddingProvider>> = None;
        let mut index: Option<QdrantIndex> = None;
        let mut embedding_vector_configured = false;
        let mut collection_needs_rebuild = false;
        let vector_fingerprint = vector_identity::fingerprint(&config);
        let stored_vector_fingerprint = db
            .get_vector_index_fingerprint(&config.qdrant.collection_name)
            .await?;
        let vector_fingerprint_changed =
            stored_vector_fingerprint.as_deref() != Some(&vector_fingerprint);

        if runtime.is_some() {
            match OpenAiCompatibleEmbeddingProvider::new(config.embedding.clone()) {
                Ok(provider) => {
                    embedding_vector_configured = true;
                    let provider: Arc<dyn EmbeddingProvider> = Arc::new(provider);
                    let mut qdrant_config = config.qdrant.clone();
                    qdrant_config.recreate_on_dimension_mismatch |= vector_fingerprint_changed;
                    match QdrantIndex::connect(&qdrant_config, config.embedding.dimensions).await {
                        Ok((connected_index, recreated)) => {
                            embedding = Some(provider);
                            index = Some(connected_index);
                            collection_needs_rebuild = recreated;
                        }
                        Err(error) => {
                            warn!(error = %error, "qdrant runtime is unavailable; continuing in degraded mode");
                        }
                    }
                }
                Err(error) => {
                    warn!(error = %error, "embedding runtime is unavailable; continuing in degraded mode");
                }
            }
        }

        let translation = TranslationService::new(TranslationDependencies {
            pool: db.pool().clone(),
            http_client: reqwest::Client::builder()
                .timeout(Duration::from_secs(120))
                .build()?,
            publisher: Arc::new(TranslationPublisherAdapter::new(
                embedding.clone(),
                index.clone(),
                ChunkingConfig {
                    max_chars: config.chunking.max_chars,
                    overlap_chars: config.chunking.overlap_chars,
                },
            )),
            concurrency: config.scheduler.max_concurrency,
            readiness: Arc::new(readiness::LibraryEmbeddingVectorReadiness {
                store: LibraryStore::new(db.clone()),
                configuration_fingerprint: vector_identity::configuration_fingerprint(&config),
            }),
        });
        let extraction = ExtractionService::new(ExtractionDependencies {
            pool: db.pool().clone(),
            http_client: reqwest::Client::builder()
                .timeout(Duration::from_secs(120))
                .build()?,
            publisher: Arc::new(ExtractionPublisherAdapter::new(db.clone(), index.clone())),
            concurrency: config.scheduler.max_concurrency,
            readiness: Arc::new(readiness::LibraryEmbeddingVectorReadiness {
                store: LibraryStore::new(db.clone()),
                configuration_fingerprint: vector_identity::configuration_fingerprint(&config),
            }),
        });
        let sync = SyncService::new(
            db.clone(),
            embedding.clone(),
            index.clone(),
            ChunkingConfig {
                max_chars: config.chunking.max_chars,
                overlap_chars: config.chunking.overlap_chars,
            },
            config.scheduler.max_concurrency,
            translation.clone(),
        );
        sync.reload_sources().await?;
        if let Err(error) = sync.validate_sources().await {
            warn!(error = %error, "source validation failed during startup; continuing without blocking service startup");
        }
        let automatic_rebuild_needed =
            sync.runtime_configured() && (collection_needs_rebuild || vector_fingerprint_changed);
        let vector_index_ready = Arc::new(AtomicBool::new(!automatic_rebuild_needed));
        if automatic_rebuild_needed {
            sync.begin_vector_index_rebuild().await?;
        }
        let query = if let (Some(embedding), Some(index)) = (embedding.clone(), index.clone()) {
            QueryService::new(
                db.clone(),
                embedding,
                index,
                config.scheduler.valkey_url.as_deref(),
                vector_identity::fingerprint(&config),
                auth.clone(),
                vector_index_ready.clone(),
            )
            .await?
        } else {
            QueryService::disabled(db.clone())
        };
        let library = LibraryService::new(
            db.clone(),
            embedding.clone(),
            index.clone(),
            crate::services::library::LibraryServiceConfig {
                chunking: ChunkingConfig {
                    max_chars: config.chunking.max_chars,
                    overlap_chars: config.chunking.overlap_chars,
                },
                file_library: config.file_library.clone(),
                valkey_url: config.scheduler.valkey_url.clone(),
                embedding_vector_configured,
                embedding_vector_configuration_fingerprint:
                    vector_identity::configuration_fingerprint(&config),
            },
            settings.clone(),
            translation.clone(),
            extraction.clone(),
        )
        .await?;
        // Before task workers resume, migrate any remaining legacy UUID
        // direct-path library files (storage_object_id IS NULL) onto the
        // content-addressed layout. This runs after LibraryService is ready and
        // before tasks.resume_pending()/translation/extraction resume, so new
        // ingestion cannot race the transition. Per-row errors are handled and
        // retried inside the migration; only a fatal selection error bubbles up
        // here. We tolerate that fatal error (log and continue) instead of
        // failing the whole startup: Docker operators must not enter the
        // container or run a manual migration, so the app keeps serving and
        // retries the migration on the next restart. This choice deliberately
        // leaves all unrelated startup behavior unchanged.
        match library.run_startup_legacy_migration().await {
            Ok(summary) => info!(
                scanned = summary.scanned,
                migrated = summary.migrated,
                already_migrated = summary.already_migrated,
                missing = summary.missing,
                invalid = summary.invalid,
                conflicts = summary.conflicts,
                errors = summary.errors,
                "startup legacy library direct-path migration complete"
            ),
            Err(error) => warn!(
                %error,
                "startup legacy library direct-path migration failed; it will retry on the next restart"
            ),
        }
        // Run missing-source cleanup before task workers resume: it
        // re-checks each terminal legacy direct-path row that the
        // migration could not bring onto the content-addressed layout
        // because its recorded source is gone from the active storage
        // backend. Qdrant availability gates the whole run so an outage
        // can never strand PostgreSQL rows without their vector points.
        // Per-row failures stay for the next startup; this only logs and
        // continues.
        let missing_summary = match library.run_startup_missing_source_cleanup().await {
            Ok(summary) => {
                info!(
                    scanned = summary.scanned,
                    confirmed_missing = summary.confirmed_missing,
                    deleted = summary.deleted,
                    still_present = summary.still_present,
                    skipped_recent_nonterminal = summary.skipped_recent_nonterminal,
                    errors = summary.errors,
                    qdrant_unavailable = summary.qdrant_unavailable,
                    "startup library missing-source cleanup complete"
                );
                summary
            }
            Err(error) => {
                warn!(
                    %error,
                    "startup library missing-source cleanup failed; it will retry on the next restart"
                );
                MissingSourceCleanupSummary::default()
            }
        };
        // Old-key cleanup is gated on no remaining legacy direct-path rows
        // and a clean missing-source cleanup so a partially-completed
        // migration cannot strand old objects that the missing-source
        // phase is still about to act on. Old-key cleanup honors its own
        // 7-day grace, backend matching, live reference checks, and
        // idempotent delete/mark; running it here is safe once the gate
        // holds. When no candidates ever existed (and no migration ever
        // recorded a legacy old-key record), this is a no-op.
        let legacy_direct_paths_remaining = library
            .store()
            .has_legacy_direct_path_files()
            .await
            .unwrap_or_else(|error| {
                warn!(
                    %error,
                    "failed to count remaining legacy direct-path rows; \
                     skipping startup old-key cleanup"
                );
                true
            });
        if missing_summary.qdrant_unavailable || missing_summary.errors > 0 {
            warn!(
                qdrant_unavailable = missing_summary.qdrant_unavailable,
                errors = missing_summary.errors,
                "skipping startup old-key cleanup because the missing-source cleanup was incomplete"
            );
        } else if legacy_direct_paths_remaining {
            info!(
                "skipping startup old-key cleanup because legacy direct-path rows remain; \
                 the missing-source cleanup will revisit them on the next startup"
            );
        } else {
            match library
                .cleanup_legacy_objects(true, DEFAULT_LEGACY_CLEANUP_BATCH_SIZE)
                .await
            {
                Ok(summary) => info!(
                    scanned = summary.scanned,
                    eligible = summary.eligible,
                    deleted = summary.deleted,
                    already_missing = summary.already_missing,
                    skipped_referenced = summary.skipped_referenced,
                    skipped_backend = summary.skipped_backend,
                    errors = summary.errors,
                    "startup legacy old-key cleanup complete"
                ),
                Err(error) => warn!(
                    %error,
                    "startup legacy old-key cleanup failed; it will retry on the next restart"
                ),
            }
        }
        let source_folders = SourceFoldersService::new(db.clone(), library.clone(), sync.clone());
        library.initialize_dependency_gates().await?;
        settings.set_docling_settings_observer(Some(Arc::new({
            let library = library.clone();
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
        let document_store = DocumentStoreService::new(db.clone(), index.clone(), library.clone());
        document_store.resume_pending();
        let tasks = TaskService::new(
            db.clone(),
            namespace.clone(),
            document_store.clone(),
            library.clone(),
            sync.clone(),
            source_folders.clone(),
            translation.clone(),
            task_worker_capacity(&config),
        );
        tasks.resume_pending();
        tasks.start_maintenance();
        translation.resume().await?;
        extraction.resume().await?;
        if let Err(error) = db.delete_expired_rerank_item_scores(30).await {
            warn!(error = %error, "failed to prune expired rerank item scores during startup");
        }
        if automatic_rebuild_needed {
            vector_rebuild::spawn(
                sync.clone(),
                db.clone(),
                index
                    .clone()
                    .expect("automatic vector rebuild requires a qdrant index"),
                config.clone(),
                vector_fingerprint,
                vector_fingerprint_changed && !collection_needs_rebuild,
                vector_index_ready,
            );
        }

        Ok(Self {
            config,
            db,
            auth,
            personal_access_tokens,
            namespace,
            query,
            sync,
            settings,
            library,
            source_folders,
            document_store,
            translation,
            extraction,
            tasks,
            browser_sessions,
        })
    }
}

pub(crate) fn task_worker_capacity(config: &Config) -> usize {
    // scheduler.max_concurrency is the single effective control for the shared
    // task worker pool (including URL imports) and the scheduler fan-out
    // (translation / extraction / sync). file_library ingest and URL-import
    // concurrency remain stored/validated for backward compatibility but do not
    // cap the task worker pool.
    crate::services::tasks::normalize_task_worker_concurrency(config.scheduler.max_concurrency)
}

#[cfg(test)]
mod tests {
    use super::task_worker_capacity;
    use crate::config::Config;

    #[test]
    fn task_worker_capacity_uses_scheduler_not_file_library() {
        let mut config = Config::default();
        config.scheduler.max_concurrency = 8;
        config.file_library.ingest_concurrency = 1;
        config.file_library.url_import_concurrency = 1;
        assert_eq!(task_worker_capacity(&config), 8);

        // Changing file_library values must not affect capacity.
        config.file_library.ingest_concurrency = 100;
        config.file_library.url_import_concurrency = 100;
        assert_eq!(task_worker_capacity(&config), 8);

        // Scheduler drives capacity independently.
        config.scheduler.max_concurrency = 4;
        config.file_library.ingest_concurrency = 1;
        config.file_library.url_import_concurrency = 1;
        assert_eq!(task_worker_capacity(&config), 4);
    }

    #[test]
    fn task_worker_capacity_clamps_zero_to_one() {
        let mut config = Config::default();
        config.scheduler.max_concurrency = 0;
        assert_eq!(task_worker_capacity(&config), 1);

        config.scheduler.max_concurrency = 1;
        assert_eq!(task_worker_capacity(&config), 1);
    }
}
