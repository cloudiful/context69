use std::time::Duration;

use anyhow::{Result, anyhow};
use docling_convert::DoclingRuntimeConfig;
use serde::{Deserialize, Serialize};

use crate::{serde_helpers, support::normalize::normalize_optional_string};

mod client;
mod xlsx_polling;

#[cfg(test)]
mod client_tests;

pub use client::DoclingXlsxClient;

pub const DEFAULT_DOCLING_BASE_URL: &str = "http://127.0.0.1:5001";
pub const DEFAULT_DOCLING_TIMEOUT_SECS: u64 = 120;
pub const DEFAULT_DOCLING_POLL_INTERVAL_SECS: u64 = 2;
pub const DEFAULT_DOCLING_TASK_TIMEOUT_SECS: u64 = 3600;
/// Persistent remote admission ceiling for Docling (issue #118).
///
/// Mirrors `context69_contracts::settings::DOCLING_MAX_INFLIGHT_*`: the Mac
/// mini single-RQ-worker default is 1, adjustable within 1..=32.
pub const DEFAULT_DOCLING_MAX_INFLIGHT: usize =
    context69_contracts::settings::DOCLING_MAX_INFLIGHT_DEFAULT;
pub const MIN_DOCLING_MAX_INFLIGHT: usize = context69_contracts::settings::DOCLING_MAX_INFLIGHT_MIN;
pub const MAX_DOCLING_MAX_INFLIGHT: usize = context69_contracts::settings::DOCLING_MAX_INFLIGHT_MAX;
pub(crate) const MAX_DOCLING_OUTPUT_BYTES: usize = 32 * 1024 * 1024;

fn default_docling_max_inflight() -> usize {
    DEFAULT_DOCLING_MAX_INFLIGHT
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DoclingConnectionConfig {
    pub base_url: String,
    #[serde(rename = "timeout_secs", with = "serde_helpers::seconds")]
    pub timeout: Duration,
    #[serde(rename = "poll_interval_secs", with = "serde_helpers::seconds")]
    pub poll_interval: Duration,
    #[serde(rename = "task_timeout_secs", with = "serde_helpers::seconds")]
    pub task_timeout: Duration,
    #[serde(default = "default_docling_max_inflight")]
    pub max_inflight: usize,
}

impl Default for DoclingConnectionConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_DOCLING_BASE_URL.to_string(),
            timeout: Duration::from_secs(DEFAULT_DOCLING_TIMEOUT_SECS),
            poll_interval: Duration::from_secs(DEFAULT_DOCLING_POLL_INTERVAL_SECS),
            task_timeout: Duration::from_secs(DEFAULT_DOCLING_TASK_TIMEOUT_SECS),
            max_inflight: DEFAULT_DOCLING_MAX_INFLIGHT,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DoclingVlmConfig {
    pub openai_base_url: Option<String>,
    pub api_key: Option<String>,
    pub vlm_pipeline_model: Option<String>,
    pub picture_description_model: Option<String>,
    pub code_formula_model: Option<String>,
    /// Optional Docling Serve `picture_description_preset` selection. Decoupled
    /// from the legacy VLM bundle so a preset alone does not require the
    /// `openai_base_url` / `api_key` / `vlm_pipeline_model` /
    /// `picture_description_model` / `code_formula_model` bundle. When set, the
    /// 0.3.3 adapter suppresses the legacy `picture_description_custom_config`
    /// form field so the preset wins on Docling Serve; leaving it `None`
    /// preserves the legacy custom VLM behaviour.
    pub picture_description_preset: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DoclingConfig {
    #[serde(flatten)]
    pub connection: DoclingConnectionConfig,
    pub vlm: DoclingVlmConfig,
}

pub fn build_runtime_config(config: &DoclingConfig) -> Result<DoclingRuntimeConfig> {
    let docling_base_url = api_base_url(&config.connection.base_url);
    let mut runtime = DoclingRuntimeConfig::without_vlm(docling_base_url);
    // The HTTP request timeout must be a short per-call ceiling so a slow
    // Docling submit or poll cannot pin a worker slot for the whole task
    // budget. The whole-document deadline is still enforced by the persisted
    // `deadline_at` next to the external job.
    runtime.request_timeout = Some(
        config
            .connection
            .timeout
            .min(Duration::from_secs(DEFAULT_DOCLING_TIMEOUT_SECS)),
    );
    runtime.task_timeout = Some(config.connection.task_timeout);
    let Some(vlm) = resolve_vlm_runtime_config(&config.vlm)? else {
        return Ok(runtime);
    };

    runtime.openai_base_url = vlm.openai_base_url;
    runtime.vlm_pipeline_model = vlm.vlm_pipeline_model;
    runtime.picture_description_model = vlm.picture_description_model;
    runtime.code_formula_model = vlm.code_formula_model;
    // The VLM provider key belongs to openai_api_key; api_key is the Docling
    // Serve X-Api-Key and must stay empty when no Docling auth is configured.
    runtime.openai_api_key = Some(vlm.api_key);
    Ok(runtime)
}

pub fn api_base_url(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.ends_with("/v1") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/v1")
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedDoclingVlmRuntimeConfig {
    openai_base_url: String,
    api_key: String,
    vlm_pipeline_model: String,
    picture_description_model: String,
    code_formula_model: String,
}

pub(crate) fn resolve_vlm_runtime_config(
    config: &DoclingVlmConfig,
) -> Result<Option<ResolvedDoclingVlmRuntimeConfig>> {
    let openai_base_url = normalize_optional_string(config.openai_base_url.clone());
    let api_key = normalize_optional_string(config.api_key.clone());
    let vlm_pipeline_model = normalize_optional_string(config.vlm_pipeline_model.clone());
    let picture_description_model =
        normalize_optional_string(config.picture_description_model.clone());
    let code_formula_model = normalize_optional_string(config.code_formula_model.clone());

    let fields = [
        openai_base_url.as_ref(),
        api_key.as_ref(),
        vlm_pipeline_model.as_ref(),
        picture_description_model.as_ref(),
        code_formula_model.as_ref(),
    ];
    let present_count = fields.iter().filter(|value| value.is_some()).count();
    if present_count == 0 {
        return Ok(None);
    }
    if present_count != fields.len() {
        return Err(anyhow!(
            "docling.vlm fields must be fully configured together: openai_base_url, api_key, vlm_pipeline_model, picture_description_model, code_formula_model"
        ));
    }

    Ok(Some(ResolvedDoclingVlmRuntimeConfig {
        openai_base_url: openai_base_url.expect("openai_base_url present"),
        api_key: api_key.expect("api_key present"),
        vlm_pipeline_model: vlm_pipeline_model.expect("vlm_pipeline_model present"),
        picture_description_model: picture_description_model
            .expect("picture_description_model present"),
        code_formula_model: code_formula_model.expect("code_formula_model present"),
    }))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{
        DoclingConfig, DoclingConnectionConfig, DoclingVlmConfig, api_base_url,
        build_runtime_config,
    };

    fn sample_config() -> DoclingConfig {
        DoclingConfig {
            connection: DoclingConnectionConfig {
                base_url: "http://localhost:5001".to_string(),
                timeout: Duration::from_secs(120),
                poll_interval: Duration::from_secs(2),
                task_timeout: Duration::from_secs(3600),
                max_inflight: super::DEFAULT_DOCLING_MAX_INFLIGHT,
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

    #[test]
    fn runtime_config_normalizes_v1_base_url() {
        let runtime = build_runtime_config(&sample_config()).expect("runtime");
        assert_eq!(runtime.docling_base_url, "http://localhost:5001/v1");
    }

    #[test]
    fn api_base_url_preserves_existing_v1_suffix() {
        assert_eq!(
            api_base_url("http://localhost:5001/v1"),
            "http://localhost:5001/v1"
        );
    }

    #[test]
    fn runtime_config_allows_disabling_vlm() {
        let config = DoclingConfig {
            connection: DoclingConnectionConfig {
                base_url: "http://localhost:5001".to_string(),
                timeout: Duration::from_secs(120),
                poll_interval: Duration::from_secs(2),
                task_timeout: Duration::from_secs(3600),
                max_inflight: super::DEFAULT_DOCLING_MAX_INFLIGHT,
            },
            vlm: DoclingVlmConfig::default(),
        };

        let runtime = build_runtime_config(&config).expect("runtime without vlm");
        assert_eq!(runtime.docling_base_url, "http://localhost:5001/v1");
        assert!(runtime.openai_base_url.is_empty());
        assert!(runtime.api_key.is_none());
    }

    #[test]
    fn runtime_config_keeps_request_timeout_short_and_separate_from_task_timeout() {
        let runtime = build_runtime_config(&sample_config()).expect("runtime");
        assert_eq!(
            runtime.request_timeout,
            Some(Duration::from_secs(120)),
            "per-request HTTP timeout stays short so a slow Docling call \
             cannot pin a worker slot for the whole task deadline",
        );
        assert_eq!(
            runtime.task_timeout,
            Some(Duration::from_secs(3600)),
            "task_timeout continues to bound the whole-document conversion",
        );
        assert_ne!(
            runtime.request_timeout, runtime.task_timeout,
            "request_timeout and task_timeout must be independent budgets"
        );
    }

    #[test]
    fn runtime_config_maps_vlm_key_to_openai_api_key() {
        let runtime = build_runtime_config(&sample_config()).expect("runtime");
        assert_eq!(
            runtime.openai_api_key.as_deref(),
            Some("secret"),
            "the VLM provider key must be sent as the OpenAI-compatible API key"
        );
        assert!(
            runtime.api_key.is_none(),
            "the VLM key must not leak into the Docling Serve X-Api-Key"
        );
    }

    #[test]
    fn runtime_config_requires_complete_vlm_settings() {
        let mut config = sample_config();
        config.vlm.code_formula_model = None;
        let error = build_runtime_config(&config).expect_err("missing model");
        assert!(
            error
                .to_string()
                .contains("must be fully configured together")
        );
    }

    #[test]
    fn legacy_config_files_without_picture_description_preset_deserialize() {
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
            config.connection.max_inflight,
            super::DEFAULT_DOCLING_MAX_INFLIGHT,
            "legacy files without max_inflight must default to the single-worker ceiling"
        );
    }

    #[test]
    fn legacy_config_files_without_max_inflight_default_to_single_worker() {
        let json = r#"{
            "connection": {
                "base_url": "http://localhost:5001",
                "timeout_secs": 120,
                "poll_interval_secs": 2,
                "task_timeout_secs": 3600
            },
            "vlm": {}
        }"#;

        let config: DoclingConfig = serde_json::from_str(json)
            .expect("legacy config without max_inflight should deserialize");
        assert_eq!(
            config.connection.max_inflight,
            super::DEFAULT_DOCLING_MAX_INFLIGHT
        );
    }
}
