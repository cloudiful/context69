use std::time::Duration;

use crate::{
    contracts::{
        CanonicalUpdateSearchSettingsRequest, DoclingConnectionSettingsResponse,
        DoclingSettingsResponse, DoclingSettingsSource, DoclingVlmSettingsResponse,
        SearchSettingsResponse, UpdateDoclingSettingsRequest, UpdateSearchSettingsRequest,
    },
    db::{StoredDoclingSettings, StoredSearchSettings},
    docling::{
        DEFAULT_DOCLING_MAX_INFLIGHT, DEFAULT_DOCLING_POLL_INTERVAL_SECS,
        DEFAULT_DOCLING_TASK_TIMEOUT_SECS, DEFAULT_DOCLING_TIMEOUT_SECS, DoclingConfig,
        DoclingConnectionConfig, DoclingVlmConfig, effective_ocr_preset,
    },
    domain_errors::DomainError,
    support::normalize::{normalize_optional_string, normalize_string_list},
};

/// The OCR preset a settings save persists.
///
/// An explicit selection wins. A request that omits the field keeps the stored
/// selection, and a stored row without one falls back to the default preset.
pub(super) fn resolved_ocr_preset(
    request: &UpdateDoclingSettingsRequest,
    stored: Option<&str>,
) -> String {
    match &request.ocr_preset {
        // A disabled OCR section hides the preset, so a blank submitted value
        // is treated as untouched rather than as a selection that would erase
        // the stored choice. An enabled request can never reach this branch
        // with a blank value because validation rejects it first.
        Some(Some(preset)) if !preset.trim().is_empty() => preset.trim().to_string(),
        _ => effective_ocr_preset(stored),
    }
}

/// The OCR toggle an update resolves to before validation runs.
///
/// An absent request flag keeps the stored value, and an unconfigured
/// installation defaults to OCR enabled. The resolved value decides whether the
/// hidden preset rules apply and what the save persists.
pub(super) fn effective_do_ocr(
    request: &UpdateDoclingSettingsRequest,
    stored: Option<&StoredDoclingSettings>,
) -> bool {
    request
        .do_ocr
        .or_else(|| stored.map(|settings| settings.do_ocr))
        .unwrap_or(true)
}

pub(super) fn docling_settings_from_request(
    request: &UpdateDoclingSettingsRequest,
    api_key: Option<String>,
    ocr_preset: &str,
    do_ocr: bool,
    do_picture_description: bool,
) -> StoredDoclingSettings {
    let vlm = request.vlm.as_ref();
    StoredDoclingSettings {
        base_url: request.connection.base_url.trim().to_string(),
        timeout_secs: request.connection.timeout_secs,
        poll_interval_secs: request.connection.poll_interval_secs,
        task_timeout_secs: request.connection.task_timeout_secs,
        max_inflight: request.connection.max_inflight,
        pdf_backend: None,
        images_scale: None,
        image_export_mode: Some("placeholder".to_string()),
        do_ocr,
        force_ocr: false,
        ocr_engine: Some(ocr_preset.to_string()),
        ocr_lang: normalize_string_list(Vec::new()),
        do_code_enrichment: true,
        do_formula_enrichment: true,
        do_picture_description,
        openai_base_url: vlm.and_then(|vlm| normalize_optional_string(vlm.openai_base_url.clone())),
        api_key,
        vlm_pipeline_model: vlm
            .and_then(|vlm| normalize_optional_string(vlm.vlm_pipeline_model.clone())),
        picture_description_model: vlm
            .and_then(|vlm| normalize_optional_string(vlm.picture_description_model.clone())),
        code_formula_model: vlm
            .and_then(|vlm| normalize_optional_string(vlm.code_formula_model.clone())),
        picture_description_preset: vlm
            .and_then(|vlm| normalize_optional_string(vlm.picture_description_preset.clone())),
    }
}

pub(super) fn search_settings_from_request(
    request: &UpdateSearchSettingsRequest,
    api_key: Option<String>,
) -> StoredSearchSettings {
    let canonical = CanonicalUpdateSearchSettingsRequest::from(request.clone());
    canonical_search_settings_from_request(&canonical, api_key)
}

pub(super) fn canonical_search_settings_from_request(
    request: &CanonicalUpdateSearchSettingsRequest,
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
        vector_weight: request.vector_weight,
        keyword_weight: request.keyword_weight,
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
            task_timeout_secs: DEFAULT_DOCLING_TASK_TIMEOUT_SECS,
            max_inflight: DEFAULT_DOCLING_MAX_INFLIGHT,
        },
        vlm: DoclingVlmSettingsResponse {
            openai_base_url: None,
            has_api_key: false,
            vlm_pipeline_model: None,
            picture_description_model: None,
            code_formula_model: None,
            picture_description_preset: None,
        },
        ocr_preset: effective_ocr_preset(None),
        do_ocr: true,
        do_picture_description: false,
    }
}

/// `has_api_key` is passed in rather than derived from the stored value: during
/// the transition the value may live only in the encrypted store, and presence
/// has to be answerable without opening it.
pub(super) fn search_response_from_stored(
    settings: StoredSearchSettings,
    has_api_key: bool,
) -> SearchSettingsResponse {
    SearchSettingsResponse {
        mode: settings.mode,
        rerank_enabled: settings.rerank_enabled,
        rerank_base_url: settings.rerank_base_url,
        rerank_model: settings.rerank_model,
        candidate_limit: settings.candidate_limit,
        timeout_secs: settings.timeout_secs,
        has_api_key,
        vector_weight: settings.vector_weight,
        keyword_weight: settings.keyword_weight,
    }
}

/// `has_api_key` is passed in rather than derived from the stored value, for the
/// same reason as [`search_response_from_stored`].
pub(super) fn response_from_stored(
    source: DoclingSettingsSource,
    configured: bool,
    settings: StoredDoclingSettings,
    has_api_key: bool,
) -> DoclingSettingsResponse {
    DoclingSettingsResponse {
        configured,
        source,
        connection: DoclingConnectionSettingsResponse {
            base_url: Some(settings.base_url),
            timeout_secs: settings.timeout_secs,
            poll_interval_secs: settings.poll_interval_secs,
            task_timeout_secs: settings.task_timeout_secs,
            max_inflight: settings.max_inflight,
        },
        vlm: DoclingVlmSettingsResponse {
            openai_base_url: settings.openai_base_url,
            has_api_key,
            vlm_pipeline_model: settings.vlm_pipeline_model,
            picture_description_model: settings.picture_description_model,
            code_formula_model: settings.code_formula_model,
            picture_description_preset: settings.picture_description_preset,
        },
        ocr_preset: effective_ocr_preset(settings.ocr_engine.as_deref()),
        do_ocr: settings.do_ocr,
        do_picture_description: settings.do_picture_description,
    }
}

/// The runtime Docling config, with the API key supplied by the caller because
/// during the transition it is resolved through the shared store rather than
/// read from the legacy column.
pub(super) fn config_from_stored(
    settings: StoredDoclingSettings,
    api_key: Option<String>,
) -> DoclingConfig {
    DoclingConfig {
        connection: DoclingConnectionConfig {
            base_url: settings.base_url,
            timeout: Duration::from_secs(settings.timeout_secs),
            poll_interval: Duration::from_secs(settings.poll_interval_secs),
            task_timeout: Duration::from_secs(settings.task_timeout_secs),
            max_inflight: settings.max_inflight,
        },
        vlm: DoclingVlmConfig {
            openai_base_url: settings.openai_base_url,
            api_key,
            vlm_pipeline_model: settings.vlm_pipeline_model,
            picture_description_model: settings.picture_description_model,
            code_formula_model: settings.code_formula_model,
            picture_description_preset: settings.picture_description_preset,
        },
        ocr_preset: effective_ocr_preset(settings.ocr_engine.as_deref()),
        do_ocr: settings.do_ocr,
        do_picture_description: Some(settings.do_picture_description),
    }
}

/// A stored Docling VLM bundle must be whole: the credential pair and the model
/// trio are each all-or-nothing, and the preset selection is exclusive with the
/// legacy bundle.
pub(super) fn validate_docling_vlm_shape(settings: &StoredDoclingSettings) -> anyhow::Result<()> {
    let openai_base_url = settings
        .openai_base_url
        .as_deref()
        .filter(|value| !value.trim().is_empty());
    let api_key = settings
        .api_key
        .as_deref()
        .filter(|value| !value.trim().is_empty());
    let vlm_pipeline_model = settings
        .vlm_pipeline_model
        .as_deref()
        .filter(|value| !value.trim().is_empty());
    let picture_description_model = settings
        .picture_description_model
        .as_deref()
        .filter(|value| !value.trim().is_empty());
    let code_formula_model = settings
        .code_formula_model
        .as_deref()
        .filter(|value| !value.trim().is_empty());
    let picture_description_preset = settings
        .picture_description_preset
        .as_deref()
        .filter(|value| !value.trim().is_empty());

    // Preset-only configuration: the picture-description preset is a
    // self-contained server-side selection and must not coexist with the
    // legacy OpenAI bundle. The ingest path forwards `picture_description_preset`
    // to the 0.3.3 Docling converter regardless of the VLM runtime path.
    if picture_description_preset.is_some() {
        if openai_base_url.is_some()
            || api_key.is_some()
            || vlm_pipeline_model.is_some()
            || picture_description_model.is_some()
            || code_formula_model.is_some()
        {
            return Err(DomainError::invalid_argument(
                "docling.vlm.picture_description_preset must not be combined with the legacy OpenAI VLM bundle (openai_base_url, api_key, vlm_pipeline_model, picture_description_model, code_formula_model)",
            )
            .into());
        }
        return Ok(());
    }

    let raw_auth_count = [openai_base_url, api_key]
        .into_iter()
        .filter(Option::is_some)
        .count();
    if raw_auth_count == 1 {
        return Err(DomainError::invalid_argument(
            "docling.vlm.openai_base_url and docling.vlm.api_key must be configured together",
        )
        .into());
    }

    let model_count = [
        vlm_pipeline_model,
        picture_description_model,
        code_formula_model,
    ]
    .into_iter()
    .filter(Option::is_some)
    .count();
    if model_count != 0 && model_count != 3 {
        return Err(DomainError::invalid_argument(
            "docling.vlm model fields must be fully configured together: vlm_pipeline_model, picture_description_model, code_formula_model",
        )
        .into());
    }

    let auth_configured = raw_auth_count == 2;
    if !auth_configured && model_count == 0 {
        return Ok(());
    }
    if !auth_configured {
        return Err(DomainError::invalid_argument(
            "docling.vlm.openai_base_url and docling.vlm.api_key are required when Docling VLM models are configured",
        )
        .into());
    }
    if model_count == 0 {
        return Err(DomainError::invalid_argument(
            "docling.vlm.vlm_pipeline_model, docling.vlm.picture_description_model, and docling.vlm.code_formula_model are required when Docling VLM is configured",
        )
        .into());
    }

    Ok(())
}

/// Keeps the stored VLM fields a disabled picture-description update omits.
///
/// A disabled section cannot be edited through the UI, so an update that omits
/// the whole VLM object keeps what is stored instead of clearing it. The caller
/// applies this only when `request.vlm` is absent: a present object is an
/// explicit branch selection — even when it is empty or carries only a key —
/// and is never merged with the opposite stored branch.
pub(super) fn preserve_hidden_docling_vlm(
    candidate: &mut StoredDoclingSettings,
    stored: &StoredDoclingSettings,
) {
    candidate.openai_base_url = stored.openai_base_url.clone();
    candidate.vlm_pipeline_model = stored.vlm_pipeline_model.clone();
    candidate.picture_description_model = stored.picture_description_model.clone();
    candidate.code_formula_model = stored.code_formula_model.clone();
    candidate.picture_description_preset = stored.picture_description_preset.clone();
}

/// Applies the hidden-field retention rule for a disabled picture update.
///
/// Only a request that omits both the whole VLM object and the explicit mode
/// keeps the stored configuration. Either one is the caller's branch selection,
/// even when the object is empty or carries only a key, so it is never merged.
pub(super) fn preserve_omitted_hidden_docling_vlm(
    request: &UpdateDoclingSettingsRequest,
    candidate: &mut StoredDoclingSettings,
    stored: &StoredDoclingSettings,
) {
    if request.vlm.is_none() && request.vlm_mode.is_none() {
        preserve_hidden_docling_vlm(candidate, stored);
    }
}

/// Skips the VLM shape check while picture description is disabled.
///
/// A disabled section keeps its stored fields untouched, so a hidden malformed
/// or partial VLM configuration must neither block a save nor be used for a
/// conversion.
pub(super) fn validate_docling_vlm_shape_if_enabled(
    settings: &StoredDoclingSettings,
    enabled: bool,
) -> anyhow::Result<()> {
    if enabled {
        validate_docling_vlm_shape(settings)
    } else {
        Ok(())
    }
}
