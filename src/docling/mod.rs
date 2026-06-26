use std::time::Duration;

use anyhow::{Context, Result};
use docling_convert::DoclingRuntimeConfig;
use serde::{Deserialize, Serialize};

use crate::serde_helpers;

mod client;

pub use client::DoclingXlsxClient;

pub const DEFAULT_DOCLING_BASE_URL: &str = "http://127.0.0.1:5001";
pub const DEFAULT_DOCLING_TIMEOUT_SECS: u64 = 120;
pub const DEFAULT_DOCLING_POLL_INTERVAL_SECS: u64 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DoclingConnectionConfig {
    pub base_url: String,
    #[serde(rename = "timeout_secs", with = "serde_helpers::seconds")]
    pub timeout: Duration,
    #[serde(rename = "poll_interval_secs", with = "serde_helpers::seconds")]
    pub poll_interval: Duration,
}

impl Default for DoclingConnectionConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_DOCLING_BASE_URL.to_string(),
            timeout: Duration::from_secs(DEFAULT_DOCLING_TIMEOUT_SECS),
            poll_interval: Duration::from_secs(DEFAULT_DOCLING_POLL_INTERVAL_SECS),
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
    Ok(DoclingRuntimeConfig {
        docling_base_url: api_base_url(&config.connection.base_url),
        openai_base_url: config.vlm.openai_base_url.clone().context(
            "docling.vlm.provider_account_key must be configured for PDF/DOCX conversion",
        )?,
        vlm_pipeline_model: config
            .vlm
            .vlm_pipeline_model
            .clone()
            .context("docling.vlm.vlm_pipeline_model must be configured for PDF/DOCX conversion")?,
        picture_description_model: config.vlm.picture_description_model.clone().context(
            "docling.vlm.picture_description_model must be configured for PDF/DOCX conversion",
        )?,
        code_formula_model: config
            .vlm
            .code_formula_model
            .clone()
            .context("docling.vlm.code_formula_model must be configured for PDF/DOCX conversion")?,
        api_key: config.vlm.api_key.clone(),
    })
}

pub fn api_base_url(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.ends_with("/v1") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/v1")
    }
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
    fn runtime_config_requires_vlm_models() {
        let mut config = sample_config();
        config.vlm.code_formula_model = None;
        let error = build_runtime_config(&config).expect_err("missing model");
        assert!(error.to_string().contains("code_formula_model"));
    }
}
