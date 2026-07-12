use std::{
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

use anyhow::Result;
use context69_translation::{TranslationDependencies, TranslationService};
use tracing::warn;

use crate::{
    chunking::ChunkingConfig,
    config::{
        Config, ConnectionConfig, EmbeddingConfig, FileLibraryConfig, QdrantConfig, SchedulerConfig,
    },
    db::{Database, StoredDoclingSettings, StoredRuntimeSettings, StoredSourceConnection},
    embedding::{EmbeddingProvider, OpenAiCompatibleEmbeddingProvider},
    qdrant_index::QdrantIndex,
    services::{
        auth::AuthService, document_store::DocumentStoreService, library::LibraryService,
        namespace::NamespaceService, personal_access_tokens::PersonalAccessTokenService,
        query::QueryService, settings::SettingsService, source_folders::SourceFoldersService,
        sync::SyncService, translation::TranslationPublisherAdapter,
    },
    source_store::SourceStore,
};

mod browser_sessions;
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
    pub browser_sessions: BrowserSessionConfig,
}

impl Context69App {
    pub async fn new(mut config: Config) -> Result<Self> {
        let db = Database::connect(&config.app_db.url).await?;
        let namespace = NamespaceService::new(db.clone());
        let auth = AuthService::new(db.clone(), config.auth.clone())?;
        let personal_access_tokens = PersonalAccessTokenService::new(db.clone(), auth.clone());
        auth.ensure_bootstrap_admin().await?;
        import_legacy_runtime_if_needed(&db, &config).await?;

        let settings = SettingsService::new(db.clone());
        let runtime = load_runtime_settings(&db).await?;
        if let Some(runtime) = &runtime {
            apply_runtime_settings(&mut config, runtime);
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
            ChunkingConfig {
                max_chars: config.chunking.max_chars,
                overlap_chars: config.chunking.overlap_chars,
            },
            settings.clone(),
            config.file_library.clone(),
            translation.clone(),
        )?;
        let source_folders = SourceFoldersService::new(db.clone(), library.clone(), sync.clone());
        library.resume_url_imports().await?;
        let document_store = DocumentStoreService::new(db.clone(), index.clone(), library.clone());
        document_store.resume_pending();
        translation.resume().await?;
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
            browser_sessions,
        })
    }
}

async fn import_legacy_runtime_if_needed(db: &Database, config: &Config) -> Result<()> {
    if db.runtime_settings_initialized().await? {
        return Ok(());
    }

    db.save_runtime_settings(&StoredRuntimeSettings {
        qdrant: crate::db::StoredRuntimeQdrantSettings {
            url: config.qdrant.url.clone(),
            collection_name: config.qdrant.collection_name.clone(),
            recreate_on_dimension_mismatch: config.qdrant.recreate_on_dimension_mismatch,
        },
        embedding: crate::db::StoredRuntimeEmbeddingSettings {
            base_url: config.embedding.base_url.clone(),
            api_key: config.embedding.api_key.clone(),
            model: config.embedding.model.clone(),
            dimensions: config.embedding.dimensions,
            timeout_secs: config.embedding.timeout.as_secs(),
        },
        scheduler: crate::db::StoredRuntimeSchedulerSettings {
            interval_secs: config.scheduler.interval.as_secs(),
            run_on_start: config.scheduler.run_on_start,
            max_concurrency: config.scheduler.max_concurrency,
            job_id: config.scheduler.job_id.clone(),
            valkey_url: config.scheduler.valkey_url.clone(),
        },
        chunking: crate::db::StoredRuntimeChunkingSettings {
            max_chars: config.chunking.max_chars,
            overlap_chars: config.chunking.overlap_chars,
        },
        file_library: crate::db::StoredRuntimeFileLibrarySettings {
            storage_root: config.file_library.storage_root.display().to_string(),
            max_upload_size_mb: config.file_library.max_upload_size_mb,
            max_upload_request_size_mb: config.file_library.max_upload_request_size_mb,
            ingest_concurrency: config.file_library.ingest_concurrency,
            pdf_pages_per_task: config.file_library.pdf_pages_per_task,
            s3: config
                .file_library
                .s3
                .as_ref()
                .map(|s3| crate::db::StoredRuntimeS3Settings {
                    endpoint: s3.endpoint.clone(),
                    region: s3.region.clone(),
                    bucket: s3.bucket.clone(),
                    prefix: s3.prefix.clone(),
                    path_style: s3.path_style,
                    access_key: s3.access_key.clone(),
                    secret_key: s3.secret_key.clone(),
                }),
        },
    })
    .await?;

    for connection in &config.connections {
        db.save_source_connection(&StoredSourceConnection {
            name: connection.name.clone(),
            database_url: connection.database_url.clone(),
        })
        .await?;
    }

    if let Some(docling) = &config.docling {
        db.save_docling_settings(&StoredDoclingSettings {
            base_url: docling.connection.base_url.clone(),
            timeout_secs: docling.connection.timeout.as_secs(),
            poll_interval_secs: docling.connection.poll_interval.as_secs(),
            pdf_backend: None,
            images_scale: None,
            image_export_mode: Some("placeholder".to_string()),
            do_ocr: true,
            force_ocr: false,
            ocr_engine: Some("rapidocr".to_string()),
            ocr_lang: Vec::new(),
            do_code_enrichment: true,
            do_formula_enrichment: true,
            do_picture_description: true,
            openai_base_url: docling.vlm.openai_base_url.clone(),
            api_key: docling.vlm.api_key.clone(),
            vlm_pipeline_model: docling.vlm.vlm_pipeline_model.clone(),
            picture_description_model: docling.vlm.picture_description_model.clone(),
            code_formula_model: docling.vlm.code_formula_model.clone(),
        })
        .await?;
    }

    SourceStore::new(db.clone())
        .seed_sources_if_empty(&config.sources)
        .await?;
    Ok(())
}

async fn load_runtime_settings(db: &Database) -> Result<Option<StoredRuntimeSettings>> {
    let Some(mut runtime) = db.get_runtime_settings().await? else {
        return Ok(None);
    };

    if let Some(grpc_url) = qdrant_grpc_url_from_rest_port(&runtime.qdrant.url) {
        warn!(
            old_url = runtime.qdrant.url,
            new_url = grpc_url,
            "upgrading legacy qdrant runtime URL from REST port to gRPC port"
        );
        runtime.qdrant.url = grpc_url;
        runtime = db.save_runtime_settings(&runtime).await?;
    }

    Ok(Some(runtime))
}

fn apply_runtime_settings(config: &mut Config, runtime: &StoredRuntimeSettings) {
    config.qdrant = QdrantConfig {
        url: runtime.qdrant.url.clone(),
        collection_name: runtime.qdrant.collection_name.clone(),
        recreate_on_dimension_mismatch: runtime.qdrant.recreate_on_dimension_mismatch,
    };
    config.embedding = EmbeddingConfig {
        base_url: runtime.embedding.base_url.clone(),
        api_key: runtime.embedding.api_key.clone(),
        model: runtime.embedding.model.clone(),
        dimensions: runtime.embedding.dimensions,
        timeout: Duration::from_secs(runtime.embedding.timeout_secs),
    };
    config.scheduler = SchedulerConfig {
        interval: Duration::from_secs(runtime.scheduler.interval_secs),
        run_on_start: runtime.scheduler.run_on_start,
        max_concurrency: runtime.scheduler.max_concurrency,
        job_id: runtime.scheduler.job_id.clone(),
        valkey_url: runtime.scheduler.valkey_url.clone(),
        execution_guard_ttl: config.scheduler.execution_guard_ttl,
        execution_guard_renew_interval: config.scheduler.execution_guard_renew_interval,
    };
    config.chunking = ChunkingConfig {
        max_chars: runtime.chunking.max_chars,
        overlap_chars: runtime.chunking.overlap_chars,
    };
    config.file_library = FileLibraryConfig {
        storage_root: PathBuf::from(&runtime.file_library.storage_root),
        max_upload_size_mb: runtime.file_library.max_upload_size_mb,
        max_upload_request_size_mb: runtime.file_library.max_upload_request_size_mb,
        ingest_concurrency: runtime.file_library.ingest_concurrency,
        pdf_pages_per_task: runtime.file_library.pdf_pages_per_task,
        s3: runtime
            .file_library
            .s3
            .as_ref()
            .map(|s3| crate::config::S3StorageConfig {
                endpoint: s3.endpoint.clone(),
                region: s3.region.clone(),
                bucket: s3.bucket.clone(),
                prefix: s3.prefix.clone(),
                path_style: s3.path_style,
                access_key: s3.access_key.clone(),
                secret_key: s3.secret_key.clone(),
            }),
    };
}

fn qdrant_grpc_url_from_rest_port(url: &str) -> Option<String> {
    let trimmed = url.trim();
    let without_trailing_slash = trimmed.strip_suffix('/').unwrap_or(trimmed);
    without_trailing_slash
        .strip_suffix(":6333")
        .map(|prefix| format!("{prefix}:6334"))
}

#[cfg(test)]
mod tests {
    use super::qdrant_grpc_url_from_rest_port;

    #[test]
    fn upgrades_qdrant_rest_port_to_grpc_port() {
        assert_eq!(
            qdrant_grpc_url_from_rest_port("http://qdrant:6333").as_deref(),
            Some("http://qdrant:6334")
        );
        assert_eq!(
            qdrant_grpc_url_from_rest_port("http://qdrant:6333/").as_deref(),
            Some("http://qdrant:6334")
        );
    }

    #[test]
    fn keeps_qdrant_grpc_port_unchanged() {
        assert_eq!(qdrant_grpc_url_from_rest_port("http://qdrant:6334"), None);
    }
}
