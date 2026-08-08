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
pub(crate) const MAX_DOCLING_OUTPUT_BYTES: usize = 32 * 1024 * 1024;

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
}

impl Default for DoclingConnectionConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_DOCLING_BASE_URL.to_string(),
            timeout: Duration::from_secs(DEFAULT_DOCLING_TIMEOUT_SECS),
            poll_interval: Duration::from_secs(DEFAULT_DOCLING_POLL_INTERVAL_SECS),
            task_timeout: Duration::from_secs(DEFAULT_DOCLING_TASK_TIMEOUT_SECS),
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
    // The HTTP request timeout must cover the whole synchronous fallback
    // conversion (convert_input still exists for legacy paths), so it shares
    // the full per-document task budget rather than the short connection
    // timeout. Async long-polls return within Docling's wait window.
    runtime.request_timeout = Some(config.connection.task_timeout);
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
            },
            vlm: DoclingVlmConfig {
                openai_base_url: Some("https://example.com/v1".to_string()),
                api_key: Some("secret".to_string()),
                vlm_pipeline_model: Some("vlm".to_string()),
                picture_description_model: Some("pic".to_string()),
                code_formula_model: Some("code".to_string()),
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
            },
            vlm: DoclingVlmConfig::default(),
        };

        let runtime = build_runtime_config(&config).expect("runtime without vlm");
        assert_eq!(runtime.docling_base_url, "http://localhost:5001/v1");
        assert!(runtime.openai_base_url.is_empty());
        assert!(runtime.api_key.is_none());
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
}
