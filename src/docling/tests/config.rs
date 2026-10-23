//! The saved Docling configuration: its defaults, its compatibility, and the
//! budgets and provider rules the client derives from it.

use std::time::Duration;

use crate::docling::{
    DEFAULT_DOCLING_MAX_INFLIGHT, DoclingConfig, DoclingConnectionConfig, DoclingVlmConfig,
    request_timeout, resolve_vlm_runtime_config,
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
