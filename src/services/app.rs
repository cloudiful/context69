use std::{path::PathBuf, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use tracing::warn;

use crate::{
    chunking::ChunkingConfig,
    config::{
        Config, ConnectionConfig, EmbeddingConfig, FileLibraryConfig, QdrantConfig, SchedulerConfig,
    },
    db::{
        Database, StoredDoclingSettings, StoredProviderAccount, StoredRuntimeSettings,
        StoredSourceConnection,
    },
    embedding::{EmbeddingProvider, OpenAiCompatibleEmbeddingProvider},
    qdrant_index::QdrantIndex,
    services::{
        auth::AuthService, library::LibraryService, namespace::NamespaceService,
        query::QueryService, settings::SettingsService, sync::SyncService,
    },
    source_store::SourceStore,
};

#[derive(Clone)]
pub struct Context69App {
    pub config: Config,
    pub db: Database,
    pub auth: AuthService,
    pub namespace: NamespaceService,
    pub query: QueryService,
    pub sync: SyncService,
    pub settings: SettingsService,
    pub library: LibraryService,
}

impl Context69App {
    pub async fn new(mut config: Config) -> Result<Self> {
        let db = Database::connect(&config.app_db.url).await?;
        let namespace = NamespaceService::new(db.clone());
        let auth = AuthService::new(db.clone(), config.auth.clone())?;
        auth.ensure_bootstrap_admin().await?;
        import_legacy_runtime_if_needed(&db, &config).await?;

        let settings = SettingsService::new(db.clone());
        let runtime = load_runtime_settings(&db).await?;
        if let Some(runtime) = &runtime {
            apply_runtime_settings(&mut config, runtime);
        }
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
        let mut recreated_collection = false;

        if let Some(runtime) = &runtime {
            match settings
                .ensure_provider_account_active(&runtime.embedding.provider_account_key, false)
                .await
            {
                Ok(embedding_account) => {
                    config.embedding = EmbeddingConfig {
                        base_url: embedding_account.base_url,
                        api_key: embedding_account.api_key,
                        model: runtime.embedding.model.clone(),
                        dimensions: runtime.embedding.dimensions,
                        timeout: Duration::from_secs(runtime.embedding.timeout_secs),
                    };

                    match OpenAiCompatibleEmbeddingProvider::new(config.embedding.clone()) {
                        Ok(provider) => {
                            let provider: Arc<dyn EmbeddingProvider> = Arc::new(provider);
                            match QdrantIndex::connect(&config.qdrant, config.embedding.dimensions)
                                .await
                            {
                                Ok((connected_index, recreated)) => {
                                    embedding = Some(provider);
                                    index = Some(connected_index);
                                    recreated_collection = recreated;
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
                Err(error) => {
                    warn!(error = %error, "embedding provider settings are invalid; continuing in degraded mode");
                }
            }
        }

        let sync = SyncService::new(
            db.clone(),
            embedding.clone(),
            index.clone(),
            ChunkingConfig {
                max_chars: config.chunking.max_chars,
                overlap_chars: config.chunking.overlap_chars,
            },
            config.scheduler.max_concurrency,
        );
        sync.reload_sources().await?;
        if let Err(error) = sync.validate_sources().await {
            warn!(error = %error, "source validation failed during startup; continuing without blocking service startup");
        }
        if recreated_collection && sync.runtime_configured() {
            sync.rebuild_index_from_db().await?;
        }
        let query = if let (Some(embedding), Some(index)) = (embedding.clone(), index.clone()) {
            QueryService::new(
                db.clone(),
                embedding,
                index,
                config.scheduler.valkey_url.as_deref(),
                config.embedding.model.clone(),
                auth.clone(),
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
        )?;
        if let Err(error) = db.delete_expired_rerank_item_scores(30).await {
            warn!(error = %error, "failed to prune expired rerank item scores during startup");
        }

        Ok(Self {
            config,
            db,
            auth,
            namespace,
            query,
            sync,
            settings,
            library,
        })
    }
}

async fn import_legacy_runtime_if_needed(db: &Database, config: &Config) -> Result<()> {
    if db.runtime_settings_initialized().await? {
        return Ok(());
    }

    let embedding_provider_key = "embedding-default".to_string();
    let docling_provider_key = if let Some(docling) = &config.docling {
        if same_provider(
            &config.embedding.base_url,
            config.embedding.api_key.as_deref(),
            docling.vlm.openai_base_url.as_deref(),
            docling.vlm.api_key.as_deref(),
        ) {
            Some(embedding_provider_key.clone())
        } else if docling.vlm.openai_base_url.is_some() || docling.vlm.api_key.is_some() {
            Some("docling-vlm".to_string())
        } else {
            None
        }
    } else {
        None
    };

    db.save_provider_account(&StoredProviderAccount {
        account_key: embedding_provider_key.clone(),
        provider_kind: "openai_compatible".to_string(),
        display_name: "Embedding Provider".to_string(),
        base_url: config.embedding.base_url.clone(),
        api_key: config.embedding.api_key.clone(),
        disabled_at: None,
    })
    .await?;

    if let (Some(docling), Some(account_key)) = (&config.docling, &docling_provider_key)
        && account_key != &embedding_provider_key
    {
        db.save_provider_account(&StoredProviderAccount {
            account_key: account_key.clone(),
            provider_kind: "openai_compatible".to_string(),
            display_name: "Docling VLM Provider".to_string(),
            base_url: docling
                .vlm
                .openai_base_url
                .clone()
                .context("docling legacy openai_base_url is required for import")?,
            api_key: docling.vlm.api_key.clone(),
            disabled_at: None,
        })
        .await?;
    }

    db.save_runtime_settings(&StoredRuntimeSettings {
        qdrant: crate::db::StoredRuntimeQdrantSettings {
            url: config.qdrant.url.clone(),
            collection_name: config.qdrant.collection_name.clone(),
            recreate_on_dimension_mismatch: config.qdrant.recreate_on_dimension_mismatch,
        },
        embedding: crate::db::StoredRuntimeEmbeddingSettings {
            provider_account_key: embedding_provider_key,
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
            provider_account_key: docling_provider_key,
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
    };
}

fn qdrant_grpc_url_from_rest_port(url: &str) -> Option<String> {
    let trimmed = url.trim();
    let without_trailing_slash = trimmed.strip_suffix('/').unwrap_or(trimmed);
    without_trailing_slash
        .strip_suffix(":6333")
        .map(|prefix| format!("{prefix}:6334"))
}

fn same_provider(
    embedding_base_url: &str,
    embedding_api_key: Option<&str>,
    docling_base_url: Option<&str>,
    docling_api_key: Option<&str>,
) -> bool {
    let Some(docling_base_url) = docling_base_url else {
        return false;
    };
    embedding_base_url.trim() == docling_base_url.trim()
        && normalize_optional_str(embedding_api_key) == normalize_optional_str(docling_api_key)
}

fn normalize_optional_str(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
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
