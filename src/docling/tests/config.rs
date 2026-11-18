//! The saved Docling configuration: its defaults, its compatibility, and the
//! budgets and provider rules the client derives from it.

use std::time::Duration;

use crate::docling::{
    DEFAULT_DOCLING_MAX_INFLIGHT, DEFAULT_DOCLING_OCR_PRESET, DoclingConfig,
    DoclingConnectionConfig, DoclingVlmConfig, InputFormat, complete_legacy_vlm_fields,
    effective_ocr_preset, request_timeout, resolve_enrichment_vlm,
    resolve_enrichment_vlm_for_input, resolve_vlm_runtime_config,
};

fn sample_config() -> DoclingConfig {
    DoclingConfig {
        connection: DoclingConnectionConfig {
            base_url: "http://localhost:5001".to_string(),
            timeout: Duration::from_secs(120),
            poll_interval: Duration::from_secs(2),
            task_timeout: Duration::from_secs(3600),
            max_inflight: DEFAULT_DOCLING_MAX_INFLIGHT,
        },
        vlm: DoclingVlmConfig {
            openai_base_url: Some("https://example.com/v1".to_string()),
            api_key: Some("secret".to_string()),
            vlm_pipeline_model: Some("vlm".to_string()),
            picture_description_model: Some("pic".to_string()),
            code_formula_model: Some("code".to_string()),
            picture_description_preset: None,
        },
        ocr_preset: DEFAULT_DOCLING_OCR_PRESET.to_string(),
        do_ocr: true,
        do_picture_description: Some(true),
        do_code_enrichment: false,
        do_formula_enrichment: false,
    }
}

/// Baseline configuration assertion: a saved endpoint is normalized to the
/// `.../v1` API root.
#[test]
fn a_saved_endpoint_is_normalized_to_the_api_root() {
    assert_eq!(
        crate::docling::api_base_url("http://localhost:5001"),
        "http://localhost:5001/v1"
    );
}

/// Baseline configuration assertion: an endpoint that already names the API
/// root keeps exactly one `v1` segment.
#[test]
fn an_endpoint_that_already_names_the_api_root_keeps_one_segment() {
    assert_eq!(
        crate::docling::api_base_url("http://localhost:5001/v1"),
        "http://localhost:5001/v1"
    );
}

/// Baseline configuration assertion: the VLM provider key resolves into the
/// OpenAI-compatible provider key, and the same key is never a Docling Serve
/// authentication value.
#[test]
fn the_vlm_provider_key_stays_separate_from_the_docling_endpoint() {
    let config = sample_config();
    let vlm = resolve_vlm_runtime_config(&config.vlm)
        .expect("resolve")
        .expect("the complete bundle resolves");
    assert_eq!(
        vlm.api_key, "secret",
        "the configured VLM key must reach the OpenAI-compatible provider"
    );
    assert_eq!(vlm.vlm_pipeline_model, "vlm");
    assert_eq!(vlm.picture_description_model, "pic");
    assert_eq!(vlm.code_formula_model, "code");
    assert!(
        !format!("{config:?}").contains("secret"),
        "the saved Docling configuration must never render the provider key"
    );
}

/// Baseline configuration assertion: an incomplete VLM bundle is rejected
/// instead of sending a half-configured provider request.
#[test]
fn an_incomplete_vlm_bundle_is_rejected() {
    let mut config = sample_config();
    config.vlm.code_formula_model = None;
    let error = resolve_vlm_runtime_config(&config.vlm).expect_err("missing model");
    assert!(
        error
            .to_string()
            .contains("must be fully configured together"),
        "{error}"
    );
}

/// Baseline configuration assertion: a configuration without any VLM field
/// resolves to no provider bundle at all.
#[test]
fn a_configuration_without_a_vlm_bundle_resolves_to_no_provider() {
    let config = DoclingConfig {
        connection: sample_config().connection,
        vlm: DoclingVlmConfig::default(),
        ocr_preset: DEFAULT_DOCLING_OCR_PRESET.to_string(),
        do_ocr: true,
        do_picture_description: None,
        do_code_enrichment: false,
        do_formula_enrichment: false,
    };
    assert!(
        resolve_vlm_runtime_config(&config.vlm)
            .expect("resolve")
            .is_none()
    );
}

/// Baseline configuration assertion: the request budget stays short and
/// separate from the whole-document budget.
#[test]
fn the_request_budget_is_capped_and_separate_from_the_task_budget() {
    let config = sample_config();
    assert_eq!(
        request_timeout(&config),
        Duration::from_secs(120),
        "the request budget bounds one HTTP request, never the whole conversion",
    );
    assert_eq!(
        config.connection.task_timeout,
        Duration::from_secs(3600),
        "the whole-document budget keeps following connection.task_timeout",
    );
    assert_ne!(
        request_timeout(&config),
        config.connection.task_timeout,
        "the request budget and the task budget must be independent",
    );
}

/// Baseline configuration assertion: raising the configured timeout never
/// raises the request ceiling, and never shrinks the document budget.
#[test]
fn the_request_ceiling_never_follows_the_configured_timeout() {
    let mut config = sample_config();
    config.connection.timeout = Duration::from_secs(600);
    assert_eq!(
        request_timeout(&config),
        Duration::from_secs(120),
        "the per-request ceiling stays clamped to DEFAULT_DOCLING_TIMEOUT_SECS",
    );
    assert_eq!(
        config.connection.task_timeout,
        Duration::from_secs(3600),
        "the whole-document budget must keep following connection.task_timeout",
    );
}

/// Baseline configuration assertion: a legacy configuration file without the
/// picture-description preset still deserializes.
#[test]
fn a_legacy_configuration_without_a_picture_description_preset_deserializes() {
    let json = r#"{
        "connection": {
            "base_url": "http://localhost:5001",
            "timeout_secs": 120,
            "poll_interval_secs": 2,
            "task_timeout_secs": 3600
        },
        "vlm": {
            "openai_base_url": null,
            "api_key": null,
            "vlm_pipeline_model": null,
            "picture_description_model": null,
            "code_formula_model": null
        }
    }"#;

    let config: DoclingConfig =
        serde_json::from_str(json).expect("legacy config without preset should deserialize");
    assert!(config.vlm.picture_description_preset.is_none());
    assert_eq!(
        config.connection.max_inflight, DEFAULT_DOCLING_MAX_INFLIGHT,
        "legacy files without max_inflight must default to the persistent ceiling"
    );
}

/// Baseline configuration assertion: a legacy configuration file without the
/// persistent ceiling defaults to the single worker.
#[test]
fn a_legacy_configuration_without_max_inflight_defaults_to_the_single_worker() {
    let json = r#"{
        "connection": {
            "base_url": "http://localhost:5001",
            "timeout_secs": 120,
            "poll_interval_secs": 2,
            "task_timeout_secs": 3600
        },
        "vlm": {}
    }"#;

    let config: DoclingConfig =
        serde_json::from_str(json).expect("legacy config without max_inflight should deserialize");
    assert_eq!(config.connection.max_inflight, DEFAULT_DOCLING_MAX_INFLIGHT);
}

/// Baseline configuration assertion: a configuration file without an OCR
/// preset uses the default preset.
#[test]
fn a_configuration_without_an_ocr_preset_uses_the_default_preset() {
    let config = DoclingConfig::default();
    assert_eq!(config.ocr_preset, DEFAULT_DOCLING_OCR_PRESET);

    let json = r#"{
        "connection": {
            "base_url": "http://localhost:5001",
            "timeout_secs": 120,
            "poll_interval_secs": 2,
            "task_timeout_secs": 3600
        },
        "vlm": {}
    }"#;
    let config: DoclingConfig =
        serde_json::from_str(json).expect("legacy config without an ocr preset should deserialize");
    assert_eq!(config.ocr_preset, DEFAULT_DOCLING_OCR_PRESET);
}

/// Baseline configuration assertion: an unset stored preset falls back to the
/// default, and every stored value is a real selection that is kept as it is.
#[test]
fn an_unset_stored_preset_falls_back_to_the_default() {
    assert_eq!(effective_ocr_preset(None), DEFAULT_DOCLING_OCR_PRESET);
    assert_eq!(effective_ocr_preset(Some("  ")), DEFAULT_DOCLING_OCR_PRESET);
    assert_eq!(effective_ocr_preset(Some("auto")), "auto");
    assert_eq!(effective_ocr_preset(Some("tesseract")), "tesseract");
}

/// A legacy file configuration without the toggles keeps OCR enabled and
/// picture description enabled only when it already configures VLM or a
/// preset. An explicit toggle always wins.
#[test]
fn omitted_file_toggles_keep_the_legacy_behavior() {
    let config = DoclingConfig::default();
    assert!(config.do_ocr);
    assert!(!config.effective_do_picture_description());

    let mut preset_only = config.clone();
    preset_only.vlm.picture_description_preset = Some("smolvlm".to_string());
    assert!(preset_only.effective_do_picture_description());

    preset_only.do_picture_description = Some(false);
    assert!(
        !preset_only.effective_do_picture_description(),
        "an explicit off toggle must never be coerced back to true"
    );

    let mut custom = config;
    custom.vlm.openai_base_url = Some("https://example.com/v1".to_string());
    custom.vlm.api_key = Some("secret".to_string());
    custom.vlm.vlm_pipeline_model = Some("vlm".to_string());
    custom.vlm.picture_description_model = Some("pic".to_string());
    custom.vlm.code_formula_model = Some("code".to_string());
    assert!(custom.effective_do_picture_description());
}

/// A fresh or legacy configuration requests no code or formula enrichment:
/// both toggles default to off and stay independent of OCR and picture
/// description.
#[test]
fn omitted_enrichment_toggles_default_to_off() {
    let config = DoclingConfig::default();
    assert!(!config.do_code_enrichment);
    assert!(!config.do_formula_enrichment);

    let json = r#"{
        "connection": {
            "base_url": "http://localhost:5001",
            "timeout_secs": 120,
            "poll_interval_secs": 2,
            "task_timeout_secs": 3600
        },
        "vlm": {},
        "do_picture_description": true
    }"#;
    let config: DoclingConfig = serde_json::from_str(json)
        .expect("legacy config without enrichment toggles should deserialize");
    assert!(config.effective_do_picture_description());
    assert!(!config.do_code_enrichment);
    assert!(!config.do_formula_enrichment);

    let json = r#"{
        "connection": {
            "base_url": "http://localhost:5001",
            "timeout_secs": 120,
            "poll_interval_secs": 2,
            "task_timeout_secs": 3600
        },
        "vlm": {},
        "do_code_enrichment": true,
        "do_formula_enrichment": false
    }"#;
    let config: DoclingConfig =
        serde_json::from_str(json).expect("explicit enrichment toggles should deserialize");
    assert!(config.do_code_enrichment);
    assert!(!config.do_formula_enrichment);
    assert!(
        !config.effective_do_picture_description(),
        "code enrichment must not enable picture description"
    );
}

/// A partial leftover legacy bundle must not block code or formula enrichment
/// while picture description is off. The enrichment falls back to the remote
/// server default instead of sending an incomplete provider request.
#[test]
fn a_partial_hidden_bundle_uses_the_remote_default_for_enrichment() {
    let mut code_only = sample_config();
    code_only.do_picture_description = Some(false);
    code_only.do_code_enrichment = true;
    code_only.vlm.code_formula_model = None;
    assert!(!complete_legacy_vlm_fields(&code_only.vlm));
    assert!(
        resolve_enrichment_vlm(&code_only.vlm, false, true, false)
            .expect("a partial hidden bundle is not a configuration error")
            .is_none(),
        "code enrichment must fall back to the remote server default"
    );

    let mut formula_only = sample_config();
    formula_only.do_picture_description = Some(false);
    formula_only.do_formula_enrichment = true;
    formula_only.vlm.openai_base_url = None;
    assert!(
        resolve_enrichment_vlm(&formula_only.vlm, false, false, true)
            .expect("resolve")
            .is_none(),
        "formula enrichment must fall back to the remote server default"
    );

    let mut blank_field = sample_config();
    blank_field.do_picture_description = Some(false);
    blank_field.do_code_enrichment = true;
    blank_field.vlm.vlm_pipeline_model = Some("   ".to_string());
    assert!(
        resolve_enrichment_vlm(&blank_field.vlm, false, true, false)
            .expect("resolve")
            .is_none(),
        "a blank leftover field is incomplete, not configured"
    );

    let mut no_bundle = sample_config();
    no_bundle.do_picture_description = Some(false);
    no_bundle.do_formula_enrichment = true;
    no_bundle.vlm = DoclingVlmConfig::default();
    assert!(
        resolve_enrichment_vlm(&no_bundle.vlm, false, false, true)
            .expect("resolve")
            .is_none(),
        "an absent bundle uses the remote server default"
    );
}

/// A complete legacy bundle stays usable for enrichment even while picture
/// description is off, so an existing operator configuration keeps working.
#[test]
fn a_complete_bundle_is_used_for_enrichment() {
    let mut config = sample_config();
    config.do_picture_description = Some(false);
    config.do_code_enrichment = true;
    config.do_formula_enrichment = true;

    let vlm = resolve_enrichment_vlm(&config.vlm, false, true, true)
        .expect("resolve")
        .expect("a complete bundle resolves");
    assert_eq!(vlm.api_key, "secret");
    assert_eq!(vlm.code_formula_model, "code");
}

/// Picture description keeps its existing all-or-nothing rule, so a genuine
/// configuration error is never hidden by the enrichment fallback.
#[test]
fn an_enabled_picture_description_still_rejects_a_partial_bundle() {
    let mut config = sample_config();
    config.vlm.picture_description_model = None;
    let error = resolve_enrichment_vlm(&config.vlm, true, false, false)
        .expect_err("picture description needs the whole bundle");
    assert!(
        error
            .to_string()
            .contains("must be fully configured together"),
        "{error}"
    );
}

/// With every enrichment branch off, a leftover bundle is never resolved.
#[test]
fn a_disabled_enrichment_resolves_no_bundle() {
    let config = sample_config();
    assert!(
        resolve_enrichment_vlm(&config.vlm, false, false, false)
            .expect("resolve")
            .is_none()
    );
}

/// An input format that never carries VLM enrichment resolves no bundle, even
/// with picture description and a complete bundle configured.
#[test]
fn an_unsupported_input_resolves_no_vlm_bundle() {
    let config = sample_config();
    assert!(InputFormat::Pdf.supports_vlm());
    assert!(InputFormat::Docx.supports_vlm());
    assert!(
        resolve_enrichment_vlm_for_input(InputFormat::Pdf, &config.vlm, true, true, true)
            .expect("resolve")
            .is_some()
    );
    assert!(
        resolve_enrichment_vlm_for_input(InputFormat::Xlsx, &config.vlm, true, true, true)
            .expect("resolve")
            .is_none(),
        "XLSX never carries VLM enrichment"
    );
}
