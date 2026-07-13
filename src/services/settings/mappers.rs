use std::time::Duration;

use crate::{
    contracts::{
        DoclingConnectionSettingsResponse, DoclingSettingsResponse, DoclingSettingsSource,
        DoclingVlmSettingsResponse, SearchSettingsResponse, UpdateDoclingSettingsRequest,
        UpdateSearchSettingsRequest,
    },
    db::{StoredDoclingSettings, StoredSearchSettings},
    docling::{
        DEFAULT_DOCLING_POLL_INTERVAL_SECS, DEFAULT_DOCLING_TIMEOUT_SECS, DoclingConfig,
        DoclingConnectionConfig, DoclingVlmConfig,
    },
    support::normalize::{normalize_optional_string, normalize_string_list},
};

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

pub(super) fn config_from_stored(settings: StoredDoclingSettings) -> DoclingConfig {
    DoclingConfig {
        connection: DoclingConnectionConfig {
            base_url: settings.base_url,
            timeout: Duration::from_secs(settings.timeout_secs),
            poll_interval: Duration::from_secs(settings.poll_interval_secs),
        },
        vlm: DoclingVlmConfig {
            openai_base_url: settings.openai_base_url,
            api_key: settings.api_key,
            vlm_pipeline_model: settings.vlm_pipeline_model,
            picture_description_model: settings.picture_description_model,
            code_formula_model: settings.code_formula_model,
        },
    }
}
