use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::{Value, json};

use super::DoclingConfig;

pub(super) struct DoclingVlmCustomConfigs {
    pub vlm_pipeline_custom_config: String,
    pub picture_description_custom_config: String,
    pub code_formula_custom_config: String,
}

pub(super) fn build_vlm_custom_configs(config: &DoclingConfig) -> Result<DoclingVlmCustomConfigs> {
    let api_key = config
        .vlm
        .api_key
        .as_deref()
        .context("docling.vlm.api_key is required when enrichment is enabled")?;
    let openai_base_url = config
        .vlm
        .openai_base_url
        .as_deref()
        .context("docling.vlm.openai_base_url is required when enrichment is enabled")?;
    let vlm_pipeline_model = config
        .vlm
        .vlm_pipeline_model
        .as_deref()
        .context("docling.vlm.vlm_pipeline_model is required when enrichment is enabled")?;
    let picture_description_model =
        config.vlm.picture_description_model.as_deref().context(
            "docling.vlm.picture_description_model is required when enrichment is enabled",
        )?;
    let code_formula_model = config
        .vlm
        .code_formula_model
        .as_deref()
        .context("docling.vlm.code_formula_model is required when enrichment is enabled")?;

    let picture_description_custom_config = PictureDescriptionVlmEngineOptions {
        batch_size: None,
        scale: Some(1.0),
        picture_area_threshold: None,
        classification_allow: None,
        classification_deny: None,
        classification_min_confidence: None,
        engine_options: build_engine_options(openai_base_url, api_key, picture_description_model),
        generation_config: None,
        model_spec: build_model_spec(
            picture_description_model,
            "Describe this image in a few sentences.",
            300,
        ),
        prompt: None,
    };
    let code_formula_custom_config = CodeFormulaVlmOptions {
        scale: Some(2.0),
        max_size: None,
        extract_code: Some(true),
        extract_formulas: Some(true),
        engine_options: build_engine_options(openai_base_url, api_key, code_formula_model),
        model_spec: build_model_spec(
            code_formula_model,
            "Recognize code blocks and mathematical formulas in the image. For code, output the full code; for mathematical formulas, output in LaTeX format.",
            1000,
        ),
    };
    let vlm_pipeline_custom_config = VlmConvertOptions {
        engine_options: build_engine_options(openai_base_url, api_key, vlm_pipeline_model),
        model_spec: build_model_spec(vlm_pipeline_model, "", 1000),
        scale: Some(1.0),
        max_size: None,
        batch_size: None,
        force_backend_text: true,
    };

    Ok(DoclingVlmCustomConfigs {
        vlm_pipeline_custom_config: serde_json::to_string(&vlm_pipeline_custom_config)
            .context("failed to serialize vlm pipeline config")?,
        picture_description_custom_config: serde_json::to_string(
            &picture_description_custom_config,
        )
        .context("failed to serialize picture description config")?,
        code_formula_custom_config: serde_json::to_string(&code_formula_custom_config)
            .context("failed to serialize code formula config")?,
    })
}

fn build_engine_options(base_url: &str, api_key: &str, model_name: &str) -> BaseVlmEngineOptions {
    BaseVlmEngineOptions {
        engine_type: "api_openai".to_string(),
        url: format!("{}/chat/completions", base_url.trim_end_matches('/')),
        headers: Some(json!({
            "Authorization": format!("Bearer {}", api_key),
        })),
        params: Some(json!({ "model": model_name })),
        timeout: Some(30),
        concurrency: Some(2),
    }
}

fn build_model_spec(model_name: &str, prompt: &str, max_new_tokens: i32) -> VlmModelSpec {
    VlmModelSpec {
        name: Some(model_name.to_string()),
        default_repo_id: Some(model_name.to_string()),
        revision: None,
        prompt: prompt.to_string(),
        response_format: "markdown".to_string(),
        max_new_tokens: Some(max_new_tokens),
        supported_engines: None,
        engine_overrides: None,
        api_overrides: None,
    }
}

#[derive(Serialize)]
struct VlmModelSpec {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    default_repo_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    revision: Option<String>,
    prompt: String,
    response_format: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_new_tokens: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    supported_engines: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    engine_overrides: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    api_overrides: Option<Value>,
}

#[derive(Serialize)]
struct BaseVlmEngineOptions {
    engine_type: String,
    url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    headers: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timeout: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    concurrency: Option<i32>,
}

#[derive(Serialize)]
struct PictureDescriptionVlmEngineOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    batch_size: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scale: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    picture_area_threshold: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    classification_allow: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    classification_deny: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    classification_min_confidence: Option<f64>,
    engine_options: BaseVlmEngineOptions,
    #[serde(skip_serializing_if = "Option::is_none")]
    generation_config: Option<Value>,
    model_spec: VlmModelSpec,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt: Option<String>,
}

#[derive(Serialize)]
struct CodeFormulaVlmOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    scale: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_size: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    extract_code: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    extract_formulas: Option<bool>,
    engine_options: BaseVlmEngineOptions,
    model_spec: VlmModelSpec,
}

#[derive(Serialize)]
struct VlmConvertOptions {
    engine_options: BaseVlmEngineOptions,
    model_spec: VlmModelSpec,
    #[serde(skip_serializing_if = "Option::is_none")]
    scale: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_size: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    batch_size: Option<i32>,
    force_backend_text: bool,
}
