//! The wire behaviour of the conversion submission.
//!
//! Every test runs against a local HTTP server on `127.0.0.1:0`; no test
//! ever reaches a deployed Docling Serve.

use std::sync::Arc;

use axum::{Json, Router, routing::post};
use serde_json::json;

use super::support::{Recorded, bodies, field, field_count, header_names, record, requests, serve};
use crate::docling::client::DoclingClient;
use crate::docling::error::DoclingError;
use crate::docling::models::{InputFormat, OutputFormat, SubmitRequest, VlmSettings};

/// The OCR preset every submission of this file names explicitly.
const OCR_PRESET: &str = crate::docling::DEFAULT_DOCLING_OCR_PRESET;

fn settings(base_url: &str) -> crate::docling::DoclingConfig {
    let mut settings = crate::docling::DoclingConfig::default();
    settings.connection.base_url = base_url.to_string();
    settings
}

fn submission<'a>(
    filename: &'a str,
    media_type: &'a str,
    bytes: &'a bytes::Bytes,
    input_format: InputFormat,
    output_formats: &'a [OutputFormat],
    vlm: Option<&'a VlmSettings>,
    picture_description_preset: Option<&'a str>,
    ocr_preset: &'a str,
) -> SubmitRequest<'a> {
    SubmitRequest {
        filename,
        media_type,
        bytes,
        input_format,
        output_formats,
        vlm,
        picture_description_preset,
        ocr_preset,
    }
}

fn vlm_settings() -> VlmSettings {
    VlmSettings {
        openai_base_url: "https://provider.example/v1".to_string(),
        api_key: "provider-secret".to_string(),
        vlm_pipeline_model: "pipeline-model".to_string(),
        picture_description_model: "picture-model".to_string(),
        code_formula_model: "code-model".to_string(),
    }
}

/// A submit route that records the request and answers with one task id.
async fn submit_router(recorded: Recorded, task_id: &'static str) -> Router {
    Router::new().route(
        "/v1/convert/file/async",
        post(move |request: axum::extract::Request| {
            let recorded = recorded.clone();
            async move {
                let (parts, body) = request.into_parts();
                let body = axum::body::to_bytes(body, 16 * 1024 * 1024)
                    .await
                    .expect("read the request body");
                record(&recorded, &parts, body);
                Json(json!({ "task_id": task_id }))
            }
        }),
    )
}

#[tokio::test]
async fn a_pdf_submission_sends_the_pypdfium2_backend_and_the_requested_formats() {
    let recorded: Recorded = Arc::new(std::sync::Mutex::new(Vec::new()));
    let base = serve(submit_router(recorded.clone(), "task-1").await).await;
    let client = DoclingClient::new(&settings(&base)).expect("client");
    let bytes = bytes::Bytes::from_static(b"fake pdf");
    let request = submission(
        "test.pdf",
        "application/pdf",
        &bytes,
        InputFormat::Pdf,
        &[OutputFormat::Md, OutputFormat::Text, OutputFormat::Json],
        None,
        None,
        OCR_PRESET,
    );

    let task_id = client.submit(&request).await.expect("submit");
    assert_eq!(task_id, "task-1");

    let body = &bodies(&recorded)[0];
    assert_eq!(field(body, "target_type").as_deref(), Some("inbody"));
    assert_eq!(field(body, "from_formats").as_deref(), Some("pdf"));
    assert_eq!(
        field(body, "pdf_backend").as_deref(),
        Some("pypdfium2"),
        "a PDF submission names the backend explicitly"
    );
    assert_eq!(field_count(body, "to_formats"), 3);
    assert!(body.contains("name=\"to_formats\"\r\n\r\nmd\r\n"));
    assert!(body.contains("name=\"to_formats\"\r\n\r\ntext\r\n"));
    assert!(body.contains("name=\"to_formats\"\r\n\r\njson\r\n"));
}

#[tokio::test]
async fn a_pdf_submission_sends_the_file_name_media_type_and_bytes() {
    let recorded: Recorded = Arc::new(std::sync::Mutex::new(Vec::new()));
    let base = serve(submit_router(recorded.clone(), "task-1").await).await;
    let client = DoclingClient::new(&settings(&base)).expect("client");
    let bytes = bytes::Bytes::from_static(b"fake pdf bytes");
    let request = submission(
        "test.pdf",
        "application/pdf",
        &bytes,
        InputFormat::Pdf,
        &[OutputFormat::Md],
        None,
        None,
        OCR_PRESET,
    );

    client.submit(&request).await.expect("submit");

    let body = &bodies(&recorded)[0];
    assert!(
        body.contains("name=\"files\"; filename=\"test.pdf\""),
        "{body}"
    );
    assert!(body.contains("Content-Type: application/pdf"), "{body}");
    assert!(body.contains("fake pdf bytes"), "{body}");
}

#[tokio::test]
async fn a_docx_submission_never_sends_the_pdf_backend() {
    let recorded: Recorded = Arc::new(std::sync::Mutex::new(Vec::new()));
    let base = serve(submit_router(recorded.clone(), "task-1").await).await;
    let client = DoclingClient::new(&settings(&base)).expect("client");
    let bytes = bytes::Bytes::from_static(b"fake docx");
    let request = submission(
        "test.docx",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        &bytes,
        InputFormat::Docx,
        &[OutputFormat::Md, OutputFormat::Text, OutputFormat::Json],
        None,
        None,
        OCR_PRESET,
    );

    client.submit(&request).await.expect("submit");

    let body = &bodies(&recorded)[0];
    assert_eq!(field(body, "from_formats").as_deref(), Some("docx"));
    assert_eq!(field(body, "pdf_backend"), None);
}

#[tokio::test]
async fn an_xlsx_submission_requests_only_json_and_no_vlm_fields() {
    let recorded: Recorded = Arc::new(std::sync::Mutex::new(Vec::new()));
    let base = serve(submit_router(recorded.clone(), "task-1").await).await;
    let client = DoclingClient::new(&settings(&base)).expect("client");
    let bytes = bytes::Bytes::from_static(b"fake xlsx");
    let vlm = vlm_settings();
    let request = submission(
        "test.xlsx",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        &bytes,
        InputFormat::Xlsx,
        &[OutputFormat::Json],
        Some(&vlm),
        None,
        OCR_PRESET,
    );

    client.submit(&request).await.expect("submit");

    let body = &bodies(&recorded)[0];
    assert_eq!(field(body, "from_formats").as_deref(), Some("xlsx"));
    assert_eq!(field_count(body, "to_formats"), 1);
    assert_eq!(field(body, "pdf_backend"), None);
    assert_eq!(field(body, "vlm_pipeline_custom_config"), None);
    assert_eq!(field(body, "picture_description_custom_config"), None);
    assert_eq!(
        field(body, "ocr_preset"),
        None,
        "an XLSX conversion never carries an OCR preset"
    );
}

#[tokio::test]
async fn the_provider_bundle_sends_the_vlm_fields_and_never_an_auth_header() {
    let recorded: Recorded = Arc::new(std::sync::Mutex::new(Vec::new()));
    let base = serve(submit_router(recorded.clone(), "task-1").await).await;
    let client = DoclingClient::new(&settings(&base)).expect("client");
    let bytes = bytes::Bytes::from_static(b"fake pdf");
    let vlm = vlm_settings();
    let request = submission(
        "test.pdf",
        "application/pdf",
        &bytes,
        InputFormat::Pdf,
        &[OutputFormat::Md],
        Some(&vlm),
        None,
        OCR_PRESET,
    );

    client.submit(&request).await.expect("submit");

    let body = &bodies(&recorded)[0];
    let pipeline = field(body, "vlm_pipeline_custom_config").expect("pipeline config");
    assert!(pipeline.contains("pipeline-model"), "{pipeline}");
    let code_formula = field(body, "code_formula_custom_config").expect("code config");
    assert!(code_formula.contains("code-model"), "{code_formula}");
    let picture =
        field(body, "picture_description_custom_config").expect("picture description config");
    assert!(picture.contains("picture-model"), "{picture}");
    assert_eq!(
        field(body, "do_picture_description").as_deref(),
        Some("true")
    );
    assert_eq!(field(body, "do_code_enrichment").as_deref(), Some("true"));
    assert_eq!(
        field(body, "do_formula_enrichment").as_deref(),
        Some("true")
    );
    assert_eq!(field(body, "ocr_preset").as_deref(), Some(OCR_PRESET));
    assert_eq!(
        field(body, "ocr_engine"),
        None,
        "the deprecated ocr_engine field must not be sent next to the preset"
    );
    assert_eq!(
        field(body, "image_export_mode").as_deref(),
        Some("placeholder")
    );

    // The provider key belongs to the provider payload, never to the Docling
    // Serve authentication header.
    let recorded_request = &requests(&recorded)[0];
    assert!(
        !header_names(recorded_request).contains(&"x-api-key".to_string()),
        "the provider key must not become a Docling authentication header"
    );
    assert!(
        !recorded_request
            .headers
            .iter()
            .any(|(_, value)| value.contains("provider-secret")),
        "the provider key must not reach a request header"
    );
}

#[tokio::test]
async fn a_preset_alone_sends_only_the_preset() {
    let recorded: Recorded = Arc::new(std::sync::Mutex::new(Vec::new()));
    let base = serve(submit_router(recorded.clone(), "task-1").await).await;
    let client = DoclingClient::new(&settings(&base)).expect("client");
    let bytes = bytes::Bytes::from_static(b"fake pdf");
    let request = submission(
        "test.pdf",
        "application/pdf",
        &bytes,
        InputFormat::Pdf,
        &[OutputFormat::Md],
        None,
        Some("granite_vision"),
        OCR_PRESET,
    );

    client.submit(&request).await.expect("submit");

    let body = &bodies(&recorded)[0];
    assert_eq!(
        field(body, "picture_description_preset").as_deref(),
        Some("granite_vision")
    );
    assert_eq!(field(body, "vlm_pipeline_custom_config"), None);
    assert_eq!(field(body, "picture_description_custom_config"), None);
    assert_eq!(field(body, "code_formula_custom_config"), None);
}

#[tokio::test]
async fn a_preset_suppresses_only_the_picture_description_configuration() {
    let recorded: Recorded = Arc::new(std::sync::Mutex::new(Vec::new()));
    let base = serve(submit_router(recorded.clone(), "task-1").await).await;
    let client = DoclingClient::new(&settings(&base)).expect("client");
    let bytes = bytes::Bytes::from_static(b"fake pdf");
    let vlm = vlm_settings();
    let request = submission(
        "test.pdf",
        "application/pdf",
        &bytes,
        InputFormat::Pdf,
        &[OutputFormat::Md],
        Some(&vlm),
        Some("granite_vision"),
        OCR_PRESET,
    );

    client.submit(&request).await.expect("submit");

    let body = &bodies(&recorded)[0];
    assert_eq!(
        field(body, "picture_description_preset").as_deref(),
        Some("granite_vision")
    );
    assert_eq!(field(body, "picture_description_custom_config"), None);
    assert!(
        field(body, "vlm_pipeline_custom_config").is_some(),
        "the pipeline mapping stays"
    );
    assert!(
        field(body, "code_formula_custom_config").is_some(),
        "the code and formula mapping stays"
    );
}

#[tokio::test]
async fn a_submission_with_a_malformed_task_id_is_rejected() {
    let recorded: Recorded = Arc::new(std::sync::Mutex::new(Vec::new()));
    let base = serve(submit_router(recorded.clone(), "task 1").await).await;
    let client = DoclingClient::new(&settings(&base)).expect("client");
    let bytes = bytes::Bytes::from_static(b"fake pdf");
    let request = submission(
        "test.pdf",
        "application/pdf",
        &bytes,
        InputFormat::Pdf,
        &[OutputFormat::Md],
        None,
        None,
        OCR_PRESET,
    );

    let error = client
        .submit(&request)
        .await
        .expect_err("a spaced id is rejected");
    assert!(matches!(error, DoclingError::Validation { .. }), "{error}");
}

#[tokio::test]
async fn the_vlm_payloads_keep_their_own_provider_timeouts() {
    let recorded: Recorded = Arc::new(std::sync::Mutex::new(Vec::new()));
    let base = serve(submit_router(recorded.clone(), "task-1").await).await;
    let client = DoclingClient::new(&settings(&base)).expect("client");
    let bytes = bytes::Bytes::from_static(b"fake pdf");
    let vlm = vlm_settings();
    let request = submission(
        "test.pdf",
        "application/pdf",
        &bytes,
        InputFormat::Pdf,
        &[OutputFormat::Md],
        Some(&vlm),
        None,
        OCR_PRESET,
    );

    client.submit(&request).await.expect("submit");

    let body = &bodies(&recorded)[0];
    let pipeline: serde_json::Value =
        serde_json::from_str(&field(body, "vlm_pipeline_custom_config").expect("pipeline config"))
            .expect("the pipeline config is json");
    assert_eq!(pipeline["engine_options"]["timeout"], 30);

    let code_formula: serde_json::Value =
        serde_json::from_str(&field(body, "code_formula_custom_config").expect("code config"))
            .expect("the code config is json");
    assert_eq!(code_formula["engine_options"]["timeout"], 30);

    let picture: serde_json::Value = serde_json::from_str(
        &field(body, "picture_description_custom_config").expect("picture config"),
    )
    .expect("the picture config is json");
    assert_eq!(
        picture["engine_options"]["timeout"], 60,
        "the picture description keeps its own, longer provider timeout"
    );
}

#[tokio::test]
async fn a_configuration_without_a_vlm_bundle_sends_no_vlm_fields() {
    let recorded: Recorded = Arc::new(std::sync::Mutex::new(Vec::new()));
    let base = serve(submit_router(recorded.clone(), "task-1").await).await;
    let mut config = settings(&base);
    config.vlm = crate::docling::DoclingVlmConfig::default();
    let client = DoclingClient::new(&config).expect("client");
    let bytes = bytes::Bytes::from_static(b"fake pdf");
    let request = submission(
        "test.pdf",
        "application/pdf",
        &bytes,
        InputFormat::Pdf,
        &[OutputFormat::Md],
        None,
        None,
        OCR_PRESET,
    );

    client.submit(&request).await.expect("submit");

    let body = &bodies(&recorded)[0];
    assert_eq!(field(body, "vlm_pipeline_custom_config"), None);
    assert_eq!(field(body, "picture_description_custom_config"), None);
    assert_eq!(field(body, "code_formula_custom_config"), None);
    assert_eq!(field(body, "do_picture_description"), None);
}

/// The OCR preset is a conversion setting of its own: a PDF conversion names
/// it even when no VLM bundle is configured.
#[tokio::test]
async fn a_pdf_submission_without_a_vlm_bundle_names_the_ocr_preset() {
    let recorded: Recorded = Arc::new(std::sync::Mutex::new(Vec::new()));
    let base = serve(submit_router(recorded.clone(), "task-1").await).await;
    let client = DoclingClient::new(&settings(&base)).expect("client");
    let bytes = bytes::Bytes::from_static(b"fake pdf");
    let request = submission(
        "test.pdf",
        "application/pdf",
        &bytes,
        InputFormat::Pdf,
        &[OutputFormat::Md],
        None,
        None,
        OCR_PRESET,
    );

    client.submit(&request).await.expect("submit");

    let body = &bodies(&recorded)[0];
    assert_eq!(field(body, "ocr_preset").as_deref(), Some(OCR_PRESET));
    assert_eq!(field(body, "ocr_engine"), None);
}

/// A DOCX conversion names the OCR preset as well.
#[tokio::test]
async fn a_docx_submission_names_the_ocr_preset() {
    let recorded: Recorded = Arc::new(std::sync::Mutex::new(Vec::new()));
    let base = serve(submit_router(recorded.clone(), "task-1").await).await;
    let client = DoclingClient::new(&settings(&base)).expect("client");
    let bytes = bytes::Bytes::from_static(b"fake docx");
    let request = submission(
        "test.docx",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        &bytes,
        InputFormat::Docx,
        &[OutputFormat::Md],
        None,
        None,
        OCR_PRESET,
    );

    client.submit(&request).await.expect("submit");

    let body = &bodies(&recorded)[0];
    assert_eq!(field(body, "ocr_preset").as_deref(), Some(OCR_PRESET));
}

/// The selected preset reaches the remote service verbatim, so a custom
/// server preset is not replaced by the default.
#[tokio::test]
async fn a_selected_ocr_preset_is_sent_verbatim() {
    let recorded: Recorded = Arc::new(std::sync::Mutex::new(Vec::new()));
    let base = serve(submit_router(recorded.clone(), "task-1").await).await;
    let client = DoclingClient::new(&settings(&base)).expect("client");
    let bytes = bytes::Bytes::from_static(b"fake pdf");
    let request = submission(
        "test.pdf",
        "application/pdf",
        &bytes,
        InputFormat::Pdf,
        &[OutputFormat::Md],
        None,
        None,
        "custom.ocr-preset",
    );

    client.submit(&request).await.expect("submit");

    let body = &bodies(&recorded)[0];
    assert_eq!(
        field(body, "ocr_preset").as_deref(),
        Some("custom.ocr-preset")
    );
    assert_eq!(field_count(body, "ocr_preset"), 1);
}
