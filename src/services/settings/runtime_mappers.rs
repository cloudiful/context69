use crate::{
    contracts::{
        RuntimeChunkingSettings, RuntimeEmbeddingSettings, RuntimeFileLibrarySettings,
        RuntimeQdrantSettings, RuntimeSchedulerSettings, RuntimeSettingsResponse,
        UpdateRuntimeSettingsRequest,
    },
    db::{
        StoredRuntimeChunkingSettings, StoredRuntimeEmbeddingSettings,
        StoredRuntimeFileLibrarySettings, StoredRuntimeQdrantSettings,
        StoredRuntimeSchedulerSettings, StoredRuntimeSettings,
    },
    support::normalize::normalize_optional_string,
};

pub(super) fn runtime_settings_from_request(
    request: &UpdateRuntimeSettingsRequest,
    api_key: Option<String>,
) -> StoredRuntimeSettings {
    StoredRuntimeSettings {
        qdrant: StoredRuntimeQdrantSettings {
            url: request.qdrant.url.trim().to_string(),
            collection_name: request.qdrant.collection_name.trim().to_string(),
            recreate_on_dimension_mismatch: request.qdrant.recreate_on_dimension_mismatch,
        },
        embedding: StoredRuntimeEmbeddingSettings {
            base_url: request.embedding.base_url.trim().to_string(),
            api_key,
            model: request.embedding.model.trim().to_string(),
            dimensions: request.embedding.dimensions,
            timeout_secs: request.embedding.timeout_secs,
        },
        scheduler: StoredRuntimeSchedulerSettings {
            interval_secs: request.scheduler.interval_secs,
            run_on_start: request.scheduler.run_on_start,
            max_concurrency: request.scheduler.max_concurrency,
            job_id: request.scheduler.job_id.trim().to_string(),
            valkey_url: normalize_optional_string(request.scheduler.valkey_url.clone()),
        },
        chunking: StoredRuntimeChunkingSettings {
            max_chars: request.chunking.max_chars,
            overlap_chars: request.chunking.overlap_chars,
        },
        file_library: StoredRuntimeFileLibrarySettings {
            storage_root: request.file_library.storage_root.trim().to_string(),
            max_upload_size_mb: request.file_library.max_upload_size_mb,
            max_upload_request_size_mb: request.file_library.max_upload_request_size_mb,
            ingest_concurrency: request.file_library.ingest_concurrency,
            url_import_concurrency: request.file_library.url_import_concurrency,
            url_import_min_interval_ms: request.file_library.url_import_min_interval_ms,
            trusted_proxy_enabled: request.file_library.trusted_proxy_enabled,
            s3: request
                .file_library
                .s3
                .as_ref()
                .map(|s3| crate::db::StoredRuntimeS3Settings {
                    endpoint: s3.endpoint.trim().to_string(),
                    region: s3.region.trim().to_string(),
                    bucket: s3.bucket.trim().to_string(),
                    prefix: s3.prefix.trim_matches('/').to_string(),
                    path_style: s3.path_style,
                    access_key: s3.access_key.trim().to_string(),
                    secret_key: s3.secret_key.clone().unwrap_or_default(),
                }),
        },
    }
}

pub(super) fn runtime_settings_response(
    settings: StoredRuntimeSettings,
) -> RuntimeSettingsResponse {
    RuntimeSettingsResponse {
        qdrant: RuntimeQdrantSettings {
            url: settings.qdrant.url,
            collection_name: settings.qdrant.collection_name,
            recreate_on_dimension_mismatch: settings.qdrant.recreate_on_dimension_mismatch,
        },
        embedding: RuntimeEmbeddingSettings {
            base_url: settings.embedding.base_url,
            model: settings.embedding.model,
            dimensions: settings.embedding.dimensions,
            timeout_secs: settings.embedding.timeout_secs,
            has_api_key: settings
                .embedding
                .api_key
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty()),
        },
        scheduler: RuntimeSchedulerSettings {
            interval_secs: settings.scheduler.interval_secs,
            run_on_start: settings.scheduler.run_on_start,
            max_concurrency: settings.scheduler.max_concurrency,
            job_id: settings.scheduler.job_id,
            valkey_url: settings.scheduler.valkey_url,
        },
        chunking: RuntimeChunkingSettings {
            max_chars: settings.chunking.max_chars,
            overlap_chars: settings.chunking.overlap_chars,
        },
        file_library: RuntimeFileLibrarySettings {
            storage_root: settings.file_library.storage_root,
            max_upload_size_mb: settings.file_library.max_upload_size_mb,
            max_upload_request_size_mb: settings.file_library.max_upload_request_size_mb,
            ingest_concurrency: settings.file_library.ingest_concurrency,
            url_import_concurrency: settings.file_library.url_import_concurrency,
            url_import_min_interval_ms: settings.file_library.url_import_min_interval_ms,
            trusted_proxy_enabled: settings.file_library.trusted_proxy_enabled,
            s3: settings
                .file_library
                .s3
                .map(|s3| crate::contracts::RuntimeS3SettingsResponse {
                    endpoint: s3.endpoint,
                    region: s3.region,
                    bucket: s3.bucket,
                    prefix: s3.prefix,
                    path_style: s3.path_style,
                    access_key: s3.access_key,
                    has_secret_key: !s3.secret_key.is_empty(),
                }),
        },
    }
}

pub(super) fn default_runtime_settings_response() -> RuntimeSettingsResponse {
    let defaults = crate::config::Config::default();
    RuntimeSettingsResponse {
        qdrant: RuntimeQdrantSettings {
            url: defaults.qdrant.url,
            collection_name: defaults.qdrant.collection_name,
            recreate_on_dimension_mismatch: defaults.qdrant.recreate_on_dimension_mismatch,
        },
        embedding: RuntimeEmbeddingSettings {
            base_url: defaults.embedding.base_url,
            model: defaults.embedding.model,
            dimensions: defaults.embedding.dimensions,
            timeout_secs: defaults.embedding.timeout.as_secs(),
            has_api_key: defaults
                .embedding
                .api_key
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty()),
        },
        scheduler: RuntimeSchedulerSettings {
            interval_secs: defaults.scheduler.interval.as_secs(),
            run_on_start: defaults.scheduler.run_on_start,
            max_concurrency: defaults.scheduler.max_concurrency,
            job_id: defaults.scheduler.job_id,
            valkey_url: defaults.scheduler.valkey_url,
        },
        chunking: RuntimeChunkingSettings {
            max_chars: defaults.chunking.max_chars,
            overlap_chars: defaults.chunking.overlap_chars,
        },
        file_library: RuntimeFileLibrarySettings {
            storage_root: defaults.file_library.storage_root.display().to_string(),
            max_upload_size_mb: defaults.file_library.max_upload_size_mb,
            max_upload_request_size_mb: defaults.file_library.max_upload_request_size_mb,
            ingest_concurrency: defaults.file_library.ingest_concurrency,
            url_import_concurrency: defaults.file_library.url_import_concurrency,
            url_import_min_interval_ms: defaults.file_library.url_import_min_interval_ms,
            trusted_proxy_enabled: defaults.file_library.trusted_proxy_enabled,
            s3: defaults
                .file_library
                .s3
                .map(|s3| crate::contracts::RuntimeS3SettingsResponse {
                    endpoint: s3.endpoint,
                    region: s3.region,
                    bucket: s3.bucket,
                    prefix: s3.prefix,
                    path_style: s3.path_style,
                    access_key: s3.access_key,
                    has_secret_key: !s3.secret_key.is_empty(),
                }),
        },
    }
}
