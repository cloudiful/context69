use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::{
    contracts::{
        DoclingConnectionSettingsResponse, DoclingSettingsResponse, DoclingSettingsSource,
        DoclingVlmSettingsResponse, ProviderAccountResponse, RuntimeChunkingSettings,
        RuntimeEmbeddingSettings, RuntimeFileLibrarySettings, RuntimeQdrantSettings,
        RuntimeSchedulerSettings, RuntimeSettingsResponse, SearchSettingsResponse,
        UpdateDoclingSettingsRequest, UpdateRuntimeSettingsRequest, UpdateSearchSettingsRequest,
    },
    db::{
        StoredDoclingSettings, StoredProviderAccount, StoredRuntimeChunkingSettings,
        StoredRuntimeEmbeddingSettings, StoredRuntimeFileLibrarySettings,
        StoredRuntimeQdrantSettings, StoredRuntimeSchedulerSettings, StoredRuntimeSettings,
        StoredSearchSettings,
    },
    docling::{
        DEFAULT_DOCLING_POLL_INTERVAL_SECS, DEFAULT_DOCLING_TIMEOUT_SECS, DoclingConfig,
        DoclingConnectionConfig, DoclingVlmConfig,
    },
    support::normalize::{normalize_optional_string, normalize_string_list},
};

pub(super) fn runtime_settings_from_request(
    request: &UpdateRuntimeSettingsRequest,
) -> StoredRuntimeSettings {
    StoredRuntimeSettings {
        qdrant: StoredRuntimeQdrantSettings {
            url: request.qdrant.url.trim().to_string(),
            collection_name: request.qdrant.collection_name.trim().to_string(),
            recreate_on_dimension_mismatch: request.qdrant.recreate_on_dimension_mismatch,
        },
        embedding: StoredRuntimeEmbeddingSettings {
            provider_account_key: request.embedding.provider_account_key.trim().to_string(),
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
            pdf_pages_per_task: request.file_library.pdf_pages_per_task,
        },
    }
}

pub(super) fn docling_settings_from_request(
    request: &UpdateDoclingSettingsRequest,
    api_key: Option<String>,
) -> StoredDoclingSettings {
    StoredDoclingSettings {
        base_url: request.connection.base_url.trim().to_string(),
        timeout_secs: request.connection.timeout_secs,
        poll_interval_secs: request.connection.poll_interval_secs,
        pdf_backend: None,
        images_scale: None,
        image_export_mode: Some("placeholder".to_string()),
        do_ocr: true,
        force_ocr: false,
        ocr_engine: Some("rapidocr".to_string()),
        ocr_lang: normalize_string_list(Vec::new()),
        do_code_enrichment: true,
        do_formula_enrichment: true,
        do_picture_description: true,
        provider_account_key: normalize_optional_string(request.vlm.provider_account_key.clone()),
        openai_base_url: normalize_optional_string(request.vlm.openai_base_url.clone()),
        api_key,
        vlm_pipeline_model: normalize_optional_string(request.vlm.vlm_pipeline_model.clone()),
        picture_description_model: normalize_optional_string(
            request.vlm.picture_description_model.clone(),
        ),
        code_formula_model: normalize_optional_string(request.vlm.code_formula_model.clone()),
    }
}

pub(super) fn search_settings_from_request(
    request: &UpdateSearchSettingsRequest,
    api_key: Option<String>,
) -> StoredSearchSettings {
    StoredSearchSettings {
        mode: request.mode,
        rerank_enabled: request.rerank_enabled,
        rerank_base_url: request.rerank_base_url.trim().to_string(),
        rerank_model: request.rerank_model.trim().to_string(),
        candidate_limit: request.candidate_limit,
        timeout_secs: request.timeout_secs,
        api_key,
    }
}

pub(super) fn provider_account_response(account: StoredProviderAccount) -> ProviderAccountResponse {
    ProviderAccountResponse {
        account_key: account.account_key,
        provider_kind: account.provider_kind,
        display_name: account.display_name,
        base_url: account.base_url,
        has_api_key: account
            .api_key
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty()),
        disabled_at: account.disabled_at,
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
            provider_account_key: settings.embedding.provider_account_key,
            model: settings.embedding.model,
            dimensions: settings.embedding.dimensions,
            timeout_secs: settings.embedding.timeout_secs,
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
            pdf_pages_per_task: settings.file_library.pdf_pages_per_task,
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
            provider_account_key: String::new(),
            model: defaults.embedding.model,
            dimensions: defaults.embedding.dimensions,
            timeout_secs: defaults.embedding.timeout.as_secs(),
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
            pdf_pages_per_task: defaults.file_library.pdf_pages_per_task,
        },
    }
}

pub(super) fn unconfigured_docling_response() -> DoclingSettingsResponse {
    DoclingSettingsResponse {
        configured: false,
        source: DoclingSettingsSource::Unconfigured,
        connection: DoclingConnectionSettingsResponse {
            base_url: None,
            timeout_secs: DEFAULT_DOCLING_TIMEOUT_SECS,
            poll_interval_secs: DEFAULT_DOCLING_POLL_INTERVAL_SECS,
        },
        vlm: DoclingVlmSettingsResponse {
            provider_account_key: None,
            openai_base_url: None,
            has_api_key: false,
            vlm_pipeline_model: None,
            picture_description_model: None,
            code_formula_model: None,
        },
    }
}

pub(super) fn search_response_from_stored(
    settings: StoredSearchSettings,
) -> SearchSettingsResponse {
    SearchSettingsResponse {
        mode: settings.mode,
        rerank_enabled: settings.rerank_enabled,
        rerank_base_url: settings.rerank_base_url,
        rerank_model: settings.rerank_model,
        candidate_limit: settings.candidate_limit,
        timeout_secs: settings.timeout_secs,
        has_api_key: settings
            .api_key
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty()),
    }
}

pub(super) fn response_from_stored(
    source: DoclingSettingsSource,
    configured: bool,
    settings: StoredDoclingSettings,
) -> DoclingSettingsResponse {
    DoclingSettingsResponse {
        configured,
        source,
        connection: DoclingConnectionSettingsResponse {
            base_url: Some(settings.base_url),
            timeout_secs: settings.timeout_secs,
            poll_interval_secs: settings.poll_interval_secs,
        },
        vlm: DoclingVlmSettingsResponse {
            provider_account_key: settings.provider_account_key,
            openai_base_url: settings.openai_base_url,
            has_api_key: settings
                .api_key
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty()),
            vlm_pipeline_model: settings.vlm_pipeline_model,
            picture_description_model: settings.picture_description_model,
            code_formula_model: settings.code_formula_model,
        },
    }
}

pub(super) fn config_from_stored(
    settings: StoredDoclingSettings,
    provider: Option<StoredProviderAccount>,
) -> DoclingConfig {
    let (openai_base_url, api_key) = match provider {
        Some(account) => (Some(account.base_url), account.api_key),
        None => (settings.openai_base_url.clone(), settings.api_key.clone()),
    };

    DoclingConfig {
        connection: DoclingConnectionConfig {
            base_url: settings.base_url,
            timeout: Duration::from_secs(settings.timeout_secs),
            poll_interval: Duration::from_secs(settings.poll_interval_secs),
        },
        vlm: DoclingVlmConfig {
            openai_base_url,
            api_key,
            vlm_pipeline_model: settings.vlm_pipeline_model,
            picture_description_model: settings.picture_description_model,
            code_formula_model: settings.code_formula_model,
        },
    }
}

pub(super) fn provider_account_from_parts(
    account_key: &str,
    provider_kind: &str,
    display_name: &str,
    base_url: &str,
    api_key: Option<String>,
    disabled_at: Option<DateTime<Utc>>,
) -> StoredProviderAccount {
    StoredProviderAccount {
        account_key: account_key.to_string(),
        provider_kind: provider_kind.to_string(),
        display_name: display_name.to_string(),
        base_url: base_url.to_string(),
        api_key,
        disabled_at,
    }
}
