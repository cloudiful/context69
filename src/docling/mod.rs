//! The Docling Serve integration of this backend.
//!
//! One typed client under [`client`] owns every remote operation; the
//! settings structs below are the saved configuration that client is built
//! from. Nothing in this module reads an environment variable for a timeout
//! or a credential: the saved configuration is the only source.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::{serde_helpers, support::normalize::normalize_optional_string};

pub(crate) mod client;
pub(crate) mod error;
pub(crate) mod models;
pub(crate) mod task_id;

#[cfg(test)]
mod tests;

pub(crate) use client::DoclingClient;
pub(crate) use error::DoclingError;
pub(crate) use models::{
    ConversionStatus, ConvertedDocument, InputFormat, OutputFormat, SubmitRequest, TaskStatus,
};

pub const DEFAULT_DOCLING_BASE_URL: &str = "http://127.0.0.1:5001";
pub const DEFAULT_DOCLING_TIMEOUT_SECS: u64 = 120;
pub const DEFAULT_DOCLING_POLL_INTERVAL_SECS: u64 = 2;
pub const DEFAULT_DOCLING_TASK_TIMEOUT_SECS: u64 = 3600;
/// The remote admission ceiling Docling defaults to: one blocking conversion
/// slot. Mirrors `context69_contracts::settings::DOCLING_MAX_INFLIGHT_DEFAULT`,
/// which owns the authoritative bounds.
pub const DEFAULT_DOCLING_MAX_INFLIGHT: usize =
    context69_contracts::settings::DOCLING_MAX_INFLIGHT_DEFAULT;
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

#[derive(Clone, Default, Serialize, Deserialize)]
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
    /// client suppresses the legacy `picture_description_custom_config`
    /// form field so the preset wins on Docling Serve; leaving it `None`
    /// preserves the legacy custom VLM behaviour.
    pub picture_description_preset: Option<String>,
}

impl std::fmt::Debug for DoclingVlmConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DoclingVlmConfig")
            .field("openai_base_url", &self.openai_base_url)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("vlm_pipeline_model", &self.vlm_pipeline_model)
            .field("picture_description_model", &self.picture_description_model)
            .field("code_formula_model", &self.code_formula_model)
            .field(
                "picture_description_preset",
                &self.picture_description_preset,
            )
            .finish()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DoclingConfig {
    #[serde(flatten)]
    pub connection: DoclingConnectionConfig,
    pub vlm: DoclingVlmConfig,
}

/// Normalizes the saved endpoint into the Docling Serve API root.
pub fn api_base_url(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.ends_with("/v1") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/v1")
    }
}

/// The per-request budget of one Docling operation.
///
/// The configured connection timeout bounds a single HTTP request (the async
/// submit and every long poll), so it is clamped to the documented ceiling.
/// The whole-document budget is `task_timeout`, which the durable task policy
/// owns; the two budgets stay independent.
pub(crate) fn request_timeout(config: &DoclingConfig) -> Duration {
    config
        .connection
        .timeout
        .min(Duration::from_secs(DEFAULT_DOCLING_TIMEOUT_SECS))
}

/// Resolves the optional VLM provider bundle.
///
/// The bundle is all-or-nothing: a partially configured bundle would send an
/// incomplete VLM request, so it is rejected instead. A picture-description
/// preset is independent of the bundle and stays usable on its own.
pub(crate) fn resolve_vlm_runtime_config(
    config: &DoclingVlmConfig,
) -> Result<Option<models::VlmSettings>, DoclingError> {
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
        return Err(DoclingError::Configuration {
            message: "docling.vlm fields must be fully configured together: openai_base_url, api_key, vlm_pipeline_model, picture_description_model, code_formula_model".to_string(),
        });
    }

    Ok(Some(models::VlmSettings {
        openai_base_url: openai_base_url.expect("openai_base_url present"),
        api_key: api_key.expect("api_key present"),
        vlm_pipeline_model: vlm_pipeline_model.expect("vlm_pipeline_model present"),
        picture_description_model: picture_description_model
            .expect("picture_description_model present"),
        code_formula_model: code_formula_model.expect("code_formula_model present"),
    }))
}
