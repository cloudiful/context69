use anyhow::{Context, Result, anyhow};
use bytes::Bytes;
use reqwest::{Client, multipart};
use serde_json::Value;
use tokio::time::sleep;

use super::{
    DoclingConfig, DoclingInputKind, DoclingOutput, bool_as_string, vlm::build_vlm_custom_configs,
};

#[derive(Debug, Clone)]
pub struct DoclingParsedDocument {
    pub text: Option<String>,
    pub json: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct DoclingRequest {
    pub filename: String,
    pub media_type: String,
    pub bytes: Bytes,
    pub from_format: &'static str,
    pub outputs: Vec<DoclingOutput>,
    pub page_range: Option<(u32, u32)>,
    pub kind: DoclingInputKind,
}

#[derive(Debug, Clone)]
pub struct DoclingClient {
    http: Client,
    config: DoclingConfig,
}

impl DoclingClient {
    pub fn new(config: DoclingConfig) -> Result<Self> {
        let http = Client::builder()
            .timeout(config.connection.timeout)
            .build()
            .context("failed to build docling http client")?;
        Ok(Self { http, config })
    }

    pub async fn convert_async(&self, request: DoclingRequest) -> Result<DoclingParsedDocument> {
        let task_id = self.submit_async(&request).await?;

        loop {
            let status = self.poll_status(&task_id).await?;
            match status.as_str() {
                "success" => return self.fetch_result(&task_id).await,
                "failure" | "revoked" => {
                    return Err(anyhow!(
                        "docling task {task_id} failed with status {status}"
                    ));
                }
                _ => sleep(self.config.connection.poll_interval).await,
            }
        }
    }

    pub(crate) fn build_form(&self, request: &DoclingRequest) -> Result<multipart::Form> {
        let part = multipart::Part::stream(reqwest::Body::from(request.bytes.clone()))
            .file_name(request.filename.clone())
            .mime_str(&request.media_type)
            .context("failed to build multipart file part")?;
        let mut form = multipart::Form::new()
            .part("files", part)
            .text("from_formats", request.from_format.to_string());
        for output in &request.outputs {
            form = form.text("to_formats", output.as_str().to_string());
        }
        if let Some((start, end)) = request.page_range {
            form = form.text("page_range", start.to_string());
            form = form.text("page_range", end.to_string());
        }

        apply_docling_options(form, &self.config, request.kind)
    }

    async fn submit_async(&self, request: &DoclingRequest) -> Result<String> {
        let form = self.build_form(request)?;
        let response = self
            .http
            .post(format!(
                "{}/v1/convert/file/async",
                self.config.connection.base_url.trim_end_matches('/')
            ))
            .multipart(form)
            .send()
            .await
            .context("failed to submit docling async job")?;
        let response = response
            .error_for_status()
            .context("docling async submission returned error status")?;
        let body = response
            .json::<Value>()
            .await
            .context("failed to parse docling async submission response")?;
        let task_id = body
            .get("task_id")
            .and_then(Value::as_str)
            .context("docling async submission missing task_id")?;
        Ok(task_id.to_string())
    }

    async fn poll_status(&self, task_id: &str) -> Result<String> {
        let response = self
            .http
            .get(format!(
                "{}/v1/status/poll/{}",
                self.config.connection.base_url.trim_end_matches('/'),
                task_id
            ))
            .send()
            .await
            .context("failed to poll docling status")?;
        let response = response
            .error_for_status()
            .context("docling status polling returned error status")?;
        let body = response
            .json::<Value>()
            .await
            .context("failed to parse docling status response")?;
        body.get("task_status")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .context("docling status response missing task_status")
    }

    async fn fetch_result(&self, task_id: &str) -> Result<DoclingParsedDocument> {
        let response = self
            .http
            .get(format!(
                "{}/v1/result/{}",
                self.config.connection.base_url.trim_end_matches('/'),
                task_id
            ))
            .send()
            .await
            .context("failed to fetch docling result")?;
        let response = response
            .error_for_status()
            .context("docling result returned error status")?;
        let body = response
            .json::<Value>()
            .await
            .context("failed to parse docling result")?;
        let document = body.get("document").unwrap_or(&body);
        Ok(DoclingParsedDocument {
            text: document
                .get("text_content")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .or_else(|| {
                    document
                        .get("md_content")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned)
                }),
            json: document.get("json_content").cloned(),
        })
    }
}

fn apply_docling_options(
    mut form: multipart::Form,
    config: &DoclingConfig,
    kind: DoclingInputKind,
) -> Result<multipart::Form> {
    match kind {
        DoclingInputKind::Pdf => {
            form = apply_pdf_options(form, config);
            form = apply_enrichment_options(form, config)?;
        }
        DoclingInputKind::Docx => {
            form = apply_docx_options(form, config);
            form = apply_enrichment_options(form, config)?;
        }
        DoclingInputKind::Xlsx => {}
    }

    Ok(form)
}

fn apply_pdf_options(mut form: multipart::Form, config: &DoclingConfig) -> multipart::Form {
    form = form.text("do_ocr", bool_as_string(config.ocr.do_ocr));
    form = form.text("force_ocr", bool_as_string(config.ocr.force_ocr));
    if let Some(value) = &config.ocr.ocr_engine {
        form = form.text("ocr_engine", value.clone());
    }
    for lang in &config.ocr.ocr_lang {
        form = form.text("ocr_lang", lang.clone());
    }
    if let Some(value) = &config.conversion.pdf_backend {
        form = form.text("pdf_backend", value.clone());
    }
    apply_shared_conversion_options(form, config)
}

fn apply_docx_options(form: multipart::Form, config: &DoclingConfig) -> multipart::Form {
    apply_shared_conversion_options(form, config)
}

fn apply_shared_conversion_options(
    mut form: multipart::Form,
    config: &DoclingConfig,
) -> multipart::Form {
    if let Some(value) = config.conversion.images_scale {
        form = form.text("images_scale", value.to_string());
    }
    if let Some(value) = &config.conversion.image_export_mode {
        form = form.text("image_export_mode", value.clone());
    }
    form
}

fn apply_enrichment_options(
    mut form: multipart::Form,
    config: &DoclingConfig,
) -> Result<multipart::Form> {
    if !config.enrichment_enabled() {
        return Ok(form);
    }

    let custom = build_vlm_custom_configs(config)?;
    form = form.text(
        "do_code_enrichment",
        bool_as_string(config.enrichment.do_code_enrichment),
    );
    form = form.text(
        "do_formula_enrichment",
        bool_as_string(config.enrichment.do_formula_enrichment),
    );
    form = form.text(
        "do_picture_description",
        bool_as_string(config.enrichment.do_picture_description),
    );
    form = form.text(
        "vlm_pipeline_custom_config",
        custom.vlm_pipeline_custom_config,
    );
    form = form.text(
        "picture_description_custom_config",
        custom.picture_description_custom_config,
    );
    form = form.text(
        "code_formula_custom_config",
        custom.code_formula_custom_config,
    );

    Ok(form)
}
