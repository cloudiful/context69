use std::{path::PathBuf, time::Duration};

use anyhow::Result;
use tracing::warn;

use crate::{
    config::{
        ChunkingBudgetConfig, Config, EmbeddingConfig, FileLibraryConfig, QdrantConfig,
        RegistryQuotaConfig, SchedulerConfig,
    },
    db::{
        Database, StoredDoclingSettings, StoredRuntimeCodeEmbeddingSettings, StoredRuntimeSettings,
    },
    services::{
        secret_store::SecretStore,
        settings::code_embedding_config,
        settings::secrets::{SettingsSecrets, optional_key_patch},
        settings::stored_chunk_budget,
    },
    source_store::SourceStore,
};

/// Seeds the first-boot runtime row from deployment configuration.
///
/// The embedding, Docling VLM, and runtime S3 API keys and every source
/// connection's database URL are committed to the shared store, so a fresh
/// database starts with every credential sealed under its purpose. They are not
/// also written to a settings row: the store is the only representation, so the
/// seeded row holds no plaintext copy of a key.
pub async fn import_legacy_runtime_if_needed(
    db: &Database,
    config: &Config,
    store: &SecretStore,
) -> Result<()> {
    if db.runtime_settings_initialized().await? {
        return Ok(());
    }

    let embedding = SettingsSecrets::embedding(store.clone());
    embedding
        .commit(&optional_key_patch(config.embedding.api_key.clone()))
        .await?;
    SettingsSecrets::runtime_s3(store.clone())
        .commit(&optional_key_patch(
            config
                .file_library
                .s3
                .as_ref()
                .map(|s3| s3.secret_key.clone()),
        ))
        .await?;
    db.save_runtime_settings(&StoredRuntimeSettings {
        qdrant: crate::db::StoredRuntimeQdrantSettings {
            url: config.qdrant.url.clone(),
            collection_name: config.qdrant.collection_name.clone(),
            recreate_on_dimension_mismatch: config.qdrant.recreate_on_dimension_mismatch,
        },
        embedding: crate::db::StoredRuntimeEmbeddingSettings {
            base_url: config.embedding.base_url.clone(),
            api_key: None,
            model: config.embedding.model.clone(),
            query_model: config.embedding.query_model.clone(),
            dimensions: config.embedding.dimensions,
            timeout_secs: config.embedding.timeout.as_secs(),
            batching: config.embedding.batching,
            input_policy: config.embedding.input_policy.clone(),
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
            unit: config.chunking.unit,
            tokenizer: match &config.chunking.tokenizer_sha256 {
                Some(sha256) => crate::chunking::chunking_token::TokenizerRef::HfLocal {
                    sha256: sha256.clone(),
                },
                None => config.chunking.tokenizer.into(),
            },
            max_tokens: config.chunking.max_tokens,
            overlap_tokens: config.chunking.overlap_tokens,
        },
        file_library: crate::db::StoredRuntimeFileLibrarySettings {
            storage_root: config.file_library.storage_root.display().to_string(),
            max_upload_size_mb: config.file_library.max_upload_size_mb,
            max_upload_request_size_mb: config.file_library.max_upload_request_size_mb,
            ingest_concurrency: config.file_library.ingest_concurrency,
            url_import_concurrency: config.file_library.url_import_concurrency,
            url_import_min_interval_ms: config.file_library.url_import_min_interval_ms,
            trusted_proxy_enabled: config.file_library.trusted_proxy_enabled,
            // The registry capacity policy is seeded from the configuration, so
            // a fresh boot installs the same caps the deployment declares.
            registry_max_ids_total: Some(config.registry_quota.registry_max_ids_total),
            registry_max_ids_per_file: Some(config.registry_quota.registry_max_ids_per_file),
            registry_max_ids_per_run: Some(config.registry_quota.registry_max_ids_per_run),
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
                    secret_key: None,
                }),
        },
    })
    .await?;

    for connection in &config.connections {
        // Through the same writer the API uses, so a seeded DSN is sealed and
        // referenced exactly like an operator-supplied one.
        crate::services::sync::save_source_connection(
            db,
            store,
            &connection.name,
            &connection.database_url,
        )
        .await?;
    }

    if let Some(docling) = &config.docling {
        SettingsSecrets::docling(store.clone())
            .commit(&optional_key_patch(docling.vlm.api_key.clone()))
            .await?;
        db.save_docling_settings(&StoredDoclingSettings {
            base_url: docling.connection.base_url.clone(),
            timeout_secs: docling.connection.timeout.as_secs(),
            poll_interval_secs: docling.connection.poll_interval.as_secs(),
            task_timeout_secs: docling.connection.task_timeout.as_secs(),
            max_inflight: docling.connection.max_inflight.clamp(
                context69_contracts::settings::DOCLING_MAX_INFLIGHT_MIN,
                context69_contracts::settings::DOCLING_MAX_INFLIGHT_MAX,
            ),
            pdf_backend: None,
            images_scale: None,
            image_export_mode: Some("placeholder".to_string()),
            do_ocr: true,
            force_ocr: false,
            ocr_engine: Some(docling.ocr_preset.clone()),
            ocr_lang: Vec::new(),
            do_code_enrichment: true,
            do_formula_enrichment: true,
            do_picture_description: true,
            openai_base_url: docling.vlm.openai_base_url.clone(),
            api_key: None,
            vlm_pipeline_model: docling.vlm.vlm_pipeline_model.clone(),
            picture_description_model: docling.vlm.picture_description_model.clone(),
            code_formula_model: docling.vlm.code_formula_model.clone(),
            picture_description_preset: docling.vlm.picture_description_preset.clone(),
        })
        .await?;
    }

    SourceStore::new(db.clone())
        .seed_sources_if_empty(&config.sources)
        .await?;
    Ok(())
}

/// Whether loading runtime settings may write the legacy Qdrant URL rewrite
/// back to the settings row.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RuntimeSettingsMode {
    /// Normal boot: the rewrite is committed exactly as it is today.
    Persist,
    /// Recovery, observers, and live reloads: normalize only in memory.
    Readonly,
}

/// Loads the persisted runtime settings with the reversible keys resolved
/// through the shared store.
///
/// The store is resolved *after* the legacy REST-to-gRPC upgrade rewrite, so the
/// rewrite keeps saving the row exactly as it did and the resolved values are
/// never written back. They then live only in this in-memory value: the
/// embedding key is handed to the embedding provider and the S3 secret key to
/// object storage, and neither is logged, persisted by this function, or
/// returned by an API.
pub async fn load_runtime_settings(
    db: &Database,
    store: &SecretStore,
) -> Result<Option<StoredRuntimeSettings>> {
    load_runtime_settings_with_mode(db, store, RuntimeSettingsMode::Persist).await
}

/// Loads the persisted optional code embedding settings with the code key
/// resolved from its own purpose.
///
/// A code row that does not exist means code embedding is unconfigured. An
/// explicitly disabled row needs no provider, so its retained credential is not
/// opened: a deployment that cannot read that row still starts with code
/// embedding disabled instead of aborting on a secret the feature does not use.
/// When the row is enabled, a sealed key this deployment cannot open fails
/// closed rather than silently configuring the code runtime without a
/// credential.
pub async fn load_code_embedding_settings(
    db: &Database,
    store: &SecretStore,
) -> Result<Option<StoredRuntimeCodeEmbeddingSettings>> {
    let Some(mut code) = db.get_runtime_code_embedding_settings().await? else {
        return Ok(None);
    };
    if !code.enabled {
        return Ok(Some(code));
    }
    code.api_key = SettingsSecrets::code_embedding(store.clone())
        .resolve()
        .await?;
    Ok(Some(code))
}

/// Applies the persisted code block to the effective configuration.
///
/// A disabled row keeps no `code_embedding` block: the non-secret fields stay in
/// the database, but the live configuration does not create a provider.
pub fn apply_code_embedding_settings(
    config: &mut Config,
    code: &StoredRuntimeCodeEmbeddingSettings,
) {
    config.code_embedding = code
        .enabled
        .then(|| code_embedding_config(code, code.api_key.clone()));
}

/// The same read for paths that must not write while they initialize.
///
/// The legacy REST-to-gRPC rewrite and every secret resolution keep their
/// behavior and defaults, but the normalized row stays in memory.
pub async fn load_runtime_settings_readonly(
    db: &Database,
    store: &SecretStore,
) -> Result<Option<StoredRuntimeSettings>> {
    load_runtime_settings_with_mode(db, store, RuntimeSettingsMode::Readonly).await
}

async fn load_runtime_settings_with_mode(
    db: &Database,
    store: &SecretStore,
    mode: RuntimeSettingsMode,
) -> Result<Option<StoredRuntimeSettings>> {
    let Some(runtime) = db.get_runtime_settings().await? else {
        return Ok(None);
    };

    let (mut runtime, rewrite) = normalize_legacy_qdrant_url(runtime, mode);
    if let Some(rewrite) = rewrite {
        runtime = db.save_runtime_settings(&rewrite).await?;
    }

    // Fails closed: a sealed embedding key this deployment cannot open must not
    // be reported as no credential, which would silently start the process
    // without one.
    runtime.embedding.api_key = SettingsSecrets::embedding(store.clone()).resolve().await?;

    // The S3 secret key is read the same way, before it can reach object storage,
    // for the same reason: a sealed row this deployment cannot open has to fail
    // startup rather than quietly configure object storage without a key.
    if let Some(s3) = runtime.file_library.s3.as_mut() {
        s3.secret_key = SettingsSecrets::runtime_s3(store.clone()).resolve().await?;
    }

    Ok(Some(runtime))
}

/// Applies the legacy REST-to-gRPC Qdrant rewrite to the in-memory value and
/// returns the row to persist, if the selected mode allows a write.
///
/// The returned row is the same value the caller uses in memory, so a caller
/// that persists it still observes exactly the normalized URL.
fn normalize_legacy_qdrant_url(
    mut runtime: StoredRuntimeSettings,
    mode: RuntimeSettingsMode,
) -> (StoredRuntimeSettings, Option<StoredRuntimeSettings>) {
    let Some(grpc_url) = qdrant_grpc_url_from_rest_port(&runtime.qdrant.url) else {
        return (runtime, None);
    };
    warn!(
        old_url = runtime.qdrant.url,
        new_url = grpc_url,
        "upgrading legacy qdrant runtime URL from REST port to gRPC port"
    );
    runtime.qdrant.url = grpc_url;
    let persist = (mode == RuntimeSettingsMode::Persist).then(|| runtime.clone());
    (runtime, persist)
}

pub fn apply_runtime_settings(config: &mut Config, runtime: &StoredRuntimeSettings) {
    config.qdrant = QdrantConfig {
        url: runtime.qdrant.url.clone(),
        collection_name: runtime.qdrant.collection_name.clone(),
        recreate_on_dimension_mismatch: runtime.qdrant.recreate_on_dimension_mismatch,
    };
    config.embedding = EmbeddingConfig {
        base_url: runtime.embedding.base_url.clone(),
        api_key: runtime.embedding.api_key.clone(),
        model: runtime.embedding.model.clone(),
        // The query model and the request limits are part of the provider
        // configuration but not of the vector space: they never change the
        // identity the collection was built with.
        query_model: runtime.embedding.query_model.clone(),
        dimensions: runtime.embedding.dimensions,
        timeout: Duration::from_secs(runtime.embedding.timeout_secs),
        batching: runtime.embedding.batching,
        input_policy: runtime.embedding.input_policy.clone(),
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
    config.chunking = ChunkingBudgetConfig::from_chunk_budget(
        &stored_chunk_budget(&runtime.chunking),
        config.chunking.tokenizer_artifact_dir.clone(),
    );
    config.file_library = FileLibraryConfig {
        storage_root: PathBuf::from(&runtime.file_library.storage_root),
        max_upload_size_mb: runtime.file_library.max_upload_size_mb,
        max_upload_request_size_mb: runtime.file_library.max_upload_request_size_mb,
        ingest_concurrency: runtime.file_library.ingest_concurrency,
        url_import_concurrency: runtime.file_library.url_import_concurrency,
        url_import_min_interval_ms: runtime.file_library.url_import_min_interval_ms,
        trusted_proxy_enabled: runtime.file_library.trusted_proxy_enabled,
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
                // An S3 block whose secret key is not configured stays in the
                // effective configuration with an empty key, so the failure is
                // S3 rejecting the request rather than the service silently
                // falling back to local storage.
                secret_key: s3.secret_key.clone().unwrap_or_default(),
            }),
    };
    // The persisted registry capacity policy stays in effect: the hydration
    // keeps the stored caps instead of falling back to the deployment
    // defaults.
    let quota = runtime.file_library.registry_quota();
    config.registry_quota = RegistryQuotaConfig {
        registry_max_ids_total: quota.max_ids_total,
        registry_max_ids_per_file: quota.max_ids_per_file,
        registry_max_ids_per_run: quota.max_ids_per_run,
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
#[path = "runtime_settings_tests.rs"]
mod tests;
