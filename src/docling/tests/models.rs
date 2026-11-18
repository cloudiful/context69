//! The wire models of the Docling protocol.

use bytes::Bytes;

use crate::docling::models::{
    InputFormat, OutputFormat, ParsedResult, SubmitRequest, TaskStatus, VlmSettings,
    code_formula_options, enrichment_form_fields, parse_result, parse_task_list, parse_task_status,
    picture_description_options, vlm_pipeline_options,
};

fn vlm_settings() -> VlmSettings {
    VlmSettings {
        openai_base_url: "https://provider.example/v1".to_string(),
        api_key: "provider-secret".to_string(),
        vlm_pipeline_model: "pipeline-model".to_string(),
        picture_description_model: "picture-model".to_string(),
        code_formula_model: "code-model".to_string(),
    }
}

#[test]
fn the_canonical_task_states_match_the_fork_vocabulary() {
    for nonterminal in ["pending", "started"] {
        let status = TaskStatus::parse(nonterminal).expect("known state");
        assert!(!status.is_terminal(), "{nonterminal} is not terminal");
        assert!(!status.is_successful(), "{nonterminal} carries no result");
        assert_eq!(status.wire_name(), nonterminal);
    }
    let success = TaskStatus::parse("success").expect("known state");
    assert!(success.is_terminal());
    assert!(success.is_successful());
    for unsuccessful in ["failure", "cancelled"] {
        let status = TaskStatus::parse(unsuccessful).expect("known state");
        assert!(status.is_terminal(), "{unsuccessful} is terminal");
        assert!(!status.is_successful(), "{unsuccessful} carries no result");
        assert_eq!(status.wire_name(), unsuccessful);
    }
}

#[test]
fn the_legacy_document_states_stay_accepted_as_task_states() {
    let partial = TaskStatus::parse("partial_success").expect("legacy state");
    assert!(partial.is_terminal());
    assert!(
        partial.is_successful(),
        "a legacy partial success still carries a result"
    );
    assert_eq!(partial.wire_name(), "partial_success");

    let skipped = TaskStatus::parse("skipped").expect("legacy state");
    assert!(skipped.is_terminal());
    assert!(!skipped.is_successful(), "a skipped document has no result");
    assert_eq!(skipped.wire_name(), "skipped");
}

#[test]
fn a_successful_state_is_success_or_partial_success() {
    assert!(TaskStatus::parse("success").expect("state").is_successful());
    assert!(
        TaskStatus::parse("partial_success")
            .expect("state")
            .is_successful()
    );
    for terminal in ["failure", "skipped", "cancelled"] {
        let status = TaskStatus::parse(terminal).expect("state");
        assert!(status.is_terminal());
        assert!(!status.is_successful(), "{terminal} carries no result");
    }
}

#[test]
fn an_unknown_future_state_stays_readable_and_non_terminal() {
    let status = TaskStatus::parse("mystery_future").expect("unknown state");
    assert_eq!(status.wire_name(), "mystery_future");
    assert!(!status.is_terminal());
    assert!(!status.is_successful());
}

#[test]
fn an_empty_state_is_malformed() {
    assert!(TaskStatus::parse("").is_err());
}

#[test]
fn the_wire_spelling_is_used_instead_of_a_debug_rendering() {
    assert_eq!(
        TaskStatus::parse("partial_success")
            .expect("state")
            .wire_name(),
        "partial_success"
    );
    assert_ne!(
        TaskStatus::parse("partial_success")
            .expect("state")
            .wire_name(),
        "partialsuccess"
    );
}

#[test]
fn the_input_formats_map_to_the_wire_values() {
    assert_eq!(InputFormat::Pdf.as_from_formats(), "pdf");
    assert_eq!(InputFormat::Docx.as_from_formats(), "docx");
    assert_eq!(InputFormat::Xlsx.as_from_formats(), "xlsx");
    assert!(InputFormat::Pdf.supports_vlm());
    assert!(InputFormat::Docx.supports_vlm());
    assert!(
        !InputFormat::Xlsx.supports_vlm(),
        "an xlsx conversion carries no picture description"
    );
}

#[test]
fn the_output_formats_map_to_the_wire_values() {
    assert_eq!(OutputFormat::Md.as_api_value(), "md");
    assert_eq!(OutputFormat::Text.as_api_value(), "text");
    assert_eq!(OutputFormat::Json.as_api_value(), "json");
}

#[test]
fn a_task_status_response_keeps_the_raw_state_and_the_failure() {
    let body = br#"{
        "task_id": "task-1",
        "task_type": "convert",
        "task_status": "failure",
        "error_message": "outer",
        "failure": {
            "category": "backend_failure",
            "message": "page 3 failed",
            "retryable": false,
            "phase": "execution"
        }
    }"#;
    let status = parse_task_status(body).expect("status");
    assert_eq!(status.task_id, "task-1");
    assert_eq!(status.task_status, TaskStatus::Failure);
    assert_eq!(status.failure_message(), Some("outer"));
}

#[test]
fn a_task_status_response_without_a_state_is_malformed() {
    let body = br#"{"task_id": "task-1"}"#;
    assert!(parse_task_status(body).is_err());
}

/// The fork adds fields this client does not consume: the extra `task_type`
/// and the nested `failure.details` stay tolerated, forward-compatible text
/// stays readable, and no raw detail is stored.
#[test]
fn a_task_status_response_tolerates_extra_fields() {
    let body = br#"{
        "task_id": "task-1",
        "task_type": "convert",
        "task_status": "failure",
        "error_message": "outer",
        "failure": {
            "category": "backend_failure",
            "message": "the backend stopped",
            "retryable": true,
            "phase": "execution",
            "details": {"worker": "gpu-1", "nested": {"attempt": 2}}
        },
        "unrecognized": [1, 2, 3]
    }"#;
    let status = parse_task_status(body).expect("status");
    assert_eq!(status.task_id, "task-1");
    assert_eq!(status.task_status, TaskStatus::Failure);
    assert_eq!(status.failure_message(), Some("outer"));
    let failure = status.failure.expect("the structured failure");
    assert_eq!(failure.category, "backend_failure");
    assert_eq!(failure.phase, "execution");
    assert_eq!(failure.message, "the backend stopped");
    assert!(failure.retryable);
}

#[test]
fn a_task_status_response_with_an_empty_state_is_malformed() {
    let body = br#"{"task_id": "task-1", "task_status": ""}"#;
    assert!(parse_task_status(body).is_err());
}

#[test]
fn a_task_list_rejects_a_malformed_entry() {
    let body = br#"{
        "items": [
            {
                "task_id": "",
                "task_type": "convert",
                "task_status": "started",
                "created_at": "2026-10-01T00:00:00Z"
            }
        ],
        "has_more": false
    }"#;
    assert!(parse_task_list(body).is_err());
}

#[test]
fn a_task_list_keeps_an_unknown_state_as_text() {
    let body = br#"{
        "items": [
            {
                "task_id": "task-1",
                "task_type": "convert",
                "task_status": "unknown_future",
                "created_at": "2026-10-01T00:00:00Z"
            }
        ],
        "has_more": true
    }"#;
    let (items, has_more) = parse_task_list(body).expect("list");
    assert!(has_more);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].task_status, "unknown_future");
}

#[test]
fn a_converted_document_result_carries_the_consumed_content() {
    let body = br##"{
        "document": {
            "filename": "a.pdf",
            "md_content": "# hi",
            "text_content": "hi",
            "json_content": {"a": 1}
        },
        "status": "success",
        "errors": []
    }"##;
    match parse_result(body).expect("result") {
        ParsedResult::Document {
            markdown,
            text,
            json,
        } => {
            assert_eq!(markdown.as_deref(), Some("# hi"));
            assert_eq!(text.as_deref(), Some("hi"));
            assert_eq!(json, Some(serde_json::json!({"a": 1})));
        }
        other => panic!("expected a document result, got {other:?}"),
    }
}

#[test]
fn a_plain_document_result_carries_the_consumed_content() {
    let body = br#"{
        "filename": "a.xlsx",
        "md_content": null,
        "text_content": null,
        "json_content": {"sheets": []}
    }"#;
    match parse_result(body).expect("result") {
        ParsedResult::Document {
            markdown,
            text,
            json,
            ..
        } => {
            assert!(markdown.is_none());
            assert!(text.is_none());
            assert_eq!(json, Some(serde_json::json!({"sheets": []})));
        }
        other => panic!("expected a document result, got {other:?}"),
    }
}

/// A Markdown-only answer carries the Markdown alone: the outputs that were
/// never requested are absent, not empty.
#[test]
fn a_markdown_only_result_carries_no_other_output() {
    let body = br##"{
        "document": {
            "filename": "a.pdf",
            "md_content": "# hi"
        },
        "status": "success"
    }"##;
    match parse_result(body).expect("result") {
        ParsedResult::Document {
            markdown,
            text,
            json,
        } => {
            assert_eq!(markdown.as_deref(), Some("# hi"));
            assert!(text.is_none());
            assert!(json.is_none());
        }
        other => panic!("expected a document result, got {other:?}"),
    }
}

/// The same answer with the unrequested outputs explicitly null stays a
/// Markdown-only result.
#[test]
fn a_markdown_only_result_with_null_outputs_stays_markdown_only() {
    let body = br##"{
        "document": {
            "filename": "a.pdf",
            "md_content": "# hi",
            "text_content": null,
            "json_content": null
        },
        "status": "success"
    }"##;
    match parse_result(body).expect("result") {
        ParsedResult::Document {
            markdown,
            text,
            json,
        } => {
            assert_eq!(markdown.as_deref(), Some("# hi"));
            assert!(text.is_none());
            assert!(json.is_none());
        }
        other => panic!("expected a document result, got {other:?}"),
    }
}

#[test]
fn a_failed_task_result_is_a_structured_failure() {
    let body = br#"{
        "kind": "TaskFailureResult",
        "failure": {
            "category": "backend_failure",
            "message": "the document could not be converted",
            "retryable": false,
            "phase": "execution"
        }
    }"#;
    match parse_result(body).expect("result") {
        ParsedResult::Failed { status, message } => {
            assert_eq!(status, "backend_failure");
            assert_eq!(message, "the document could not be converted");
        }
        other => panic!("expected a failed result, got {other:?}"),
    }
}

#[test]
fn a_result_that_is_not_json_is_malformed() {
    assert!(parse_result(b"not json").is_err());
}

#[test]
fn a_direct_export_without_an_envelope_is_a_result() {
    let body = br##"{
        "filename": "a.pdf",
        "md_content": "# hi",
        "text_content": "hi",
        "json_content": {"a": 1}
    }"##;
    match parse_result(body).expect("result") {
        ParsedResult::Document {
            markdown,
            text,
            json,
        } => {
            assert_eq!(markdown.as_deref(), Some("# hi"));
            assert_eq!(text.as_deref(), Some("hi"));
            assert_eq!(json, Some(serde_json::json!({"a": 1})));
        }
        other => panic!("expected a document result, got {other:?}"),
    }
}

#[test]
fn an_empty_content_is_a_legitimate_empty_output() {
    let body = br#"{"md_content": "", "text_content": null}"#;
    match parse_result(body).expect("result") {
        ParsedResult::Document {
            markdown,
            text,
            json,
        } => {
            assert_eq!(markdown.as_deref(), Some(""));
            assert_eq!(text, None);
            assert_eq!(json, None);
        }
        other => panic!("expected a document result, got {other:?}"),
    }
}

#[test]
fn a_partial_success_envelope_yields_the_document() {
    let body = br##"{
        "document": {"filename": "a.pdf", "md_content": "# hi"},
        "status": "partial_success"
    }"##;
    match parse_result(body).expect("result") {
        ParsedResult::Document { markdown, .. } => {
            assert_eq!(markdown.as_deref(), Some("# hi"));
        }
        other => panic!("expected a document result, got {other:?}"),
    }
}

#[test]
fn an_object_without_a_document_is_not_a_result() {
    for body in [
        &br#"{}"#[..],
        &br#"{"error": "boom"}"#[..],
        &br#"{"filename": "a.pdf"}"#[..],
        &br#"{"md_content": 123}"#[..],
        &br#"{"text_content": {"a": 1}}"#[..],
    ] {
        assert!(
            parse_result(body).is_err(),
            "an unrelated object must never become an empty success: {}",
            String::from_utf8_lossy(body)
        );
    }
}

#[test]
fn an_empty_document_envelope_is_not_a_result() {
    let body = br#"{"document": {}, "status": "success"}"#;
    assert!(parse_result(body).is_err());
}

#[test]
fn a_non_success_envelope_state_is_never_a_document() {
    for status in ["pending", "started", "mystery_future"] {
        let body = format!(r##"{{"document": {{"md_content": "# hi"}}, "status": "{status}"}}"##);
        assert!(
            parse_result(body.as_bytes()).is_err(),
            "a {status} result is never a document"
        );
    }
    let body = br##"{"document": {"md_content": "# hi"}, "status": 7}"##;
    assert!(parse_result(body).is_err());
}

#[test]
fn a_non_success_envelope_state_is_a_terminal_failure() {
    for status in ["failure", "skipped", "cancelled"] {
        let body = format!(
            r##"{{"document": {{"md_content": "# hi"}}, "status": "{status}", "error_message": "the document was not converted"}}"##
        );
        match parse_result(body.as_bytes()).expect("result") {
            ParsedResult::Failed {
                status: observed,
                message,
            } => {
                assert_eq!(observed, status);
                assert_eq!(message, "the document was not converted");
            }
            other => panic!("expected a failed result, got {other:?}"),
        }
    }
}

#[test]
fn the_vlm_engine_options_carry_the_provider_binding() {
    let options = vlm_pipeline_options(&vlm_settings());
    let value = serde_json::to_value(&options).expect("json");
    assert_eq!(value["engine_options"]["engine_type"], "api_openai");
    assert_eq!(
        value["engine_options"]["url"],
        "https://provider.example/v1/chat/completions"
    );
    assert_eq!(
        value["engine_options"]["headers"]["Authorization"],
        "Bearer provider-secret"
    );
    assert_eq!(value["engine_options"]["params"]["model"], "pipeline-model");
    assert_eq!(value["engine_options"]["timeout"], 30);
    assert_eq!(value["engine_options"]["concurrency"], 2);
    assert_eq!(value["model_spec"]["response_format"], "markdown");
    assert_eq!(value["model_spec"]["max_new_tokens"], 1000);
    assert_eq!(value["force_backend_text"], true);
}

#[test]
fn the_picture_description_options_carry_their_own_model() {
    let options = picture_description_options(&vlm_settings());
    let value = serde_json::to_value(&options).expect("json");
    assert_eq!(value["model_spec"]["name"], "picture-model");
    assert_eq!(
        value["model_spec"]["prompt"],
        "Describe this image in a few sentences."
    );
    assert_eq!(value["model_spec"]["max_new_tokens"], 300);
    assert_eq!(value["engine_options"]["params"]["model"], "picture-model");
}

#[test]
fn the_code_and_formula_options_carry_their_own_model() {
    let options = code_formula_options(&vlm_settings(), true, true);
    let value = serde_json::to_value(&options).expect("json");
    assert_eq!(value["model_spec"]["name"], "code-model");
    assert_eq!(value["model_spec"]["max_new_tokens"], 1000);
    assert_eq!(value["extract_code"], true);
    assert_eq!(value["extract_formulas"], true);
    assert_eq!(value["scale"], 2.0);
}

#[test]
fn the_vlm_settings_never_render_the_provider_key() {
    let rendered = format!("{:?}", vlm_settings());
    assert!(!rendered.contains("provider-secret"), "{rendered}");
    assert!(rendered.contains("<redacted>"), "{rendered}");
}

/// The credential-bearing VLM wire models never render the provider key, the
/// authorization header, or the provider endpoint through Debug, while the
/// serialized wire value keeps all three exactly.
#[test]
fn the_vlm_wire_models_never_render_the_provider_credential() {
    let vlm = vlm_settings();
    let renderings = [
        format!("{:?}", vlm_pipeline_options(&vlm)),
        format!("{:?}", picture_description_options(&vlm)),
        format!("{:?}", code_formula_options(&vlm, true, true)),
    ];

    for rendered in &renderings {
        assert!(
            !rendered.contains("provider-secret"),
            "the provider key must never reach a debug rendering: {rendered}"
        );
        assert!(
            !rendered.contains("Authorization"),
            "the authorization header must never reach a debug rendering: {rendered}"
        );
        assert!(
            !rendered.contains("Bearer"),
            "the bearer scheme must never reach a debug rendering: {rendered}"
        );
        assert!(
            !rendered.contains("provider.example"),
            "the provider endpoint must never reach a debug rendering: {rendered}"
        );
        assert!(
            rendered.contains("VlmEngineOptions"),
            "the engine metadata stays visible: {rendered}"
        );
        assert!(
            rendered.contains("api_openai"),
            "the non-sensitive engine type stays visible: {rendered}"
        );
    }
}

/// The redaction is confined to Debug: the outgoing wire value still carries
/// the real authorization header, the provider endpoint, the model, and the
/// per-purpose budgets.
#[test]
fn the_vlm_wire_serialization_keeps_the_provider_credential() {
    let vlm = vlm_settings();

    let pipeline = serde_json::to_value(vlm_pipeline_options(&vlm)).expect("json");
    assert_eq!(
        pipeline["engine_options"]["headers"]["Authorization"],
        "Bearer provider-secret"
    );
    assert_eq!(
        pipeline["engine_options"]["url"],
        "https://provider.example/v1/chat/completions"
    );
    assert_eq!(pipeline["engine_options"]["timeout"], 30);

    let picture = serde_json::to_value(picture_description_options(&vlm)).expect("json");
    assert_eq!(
        picture["engine_options"]["headers"]["Authorization"],
        "Bearer provider-secret"
    );
    assert_eq!(picture["engine_options"]["timeout"], 60);

    let code_formula = serde_json::to_value(code_formula_options(&vlm, true, true)).expect("json");
    assert_eq!(
        code_formula["engine_options"]["headers"]["Authorization"],
        "Bearer provider-secret"
    );
    assert_eq!(code_formula["engine_options"]["timeout"], 30);
}

fn enrichment_request<'a>(
    bytes: &'a Bytes,
    outputs: &'a [OutputFormat],
    case: EnrichmentCase<'a>,
) -> SubmitRequest<'a> {
    SubmitRequest {
        filename: "a.pdf",
        media_type: "application/pdf",
        bytes,
        input_format: case.input_format,
        output_formats: outputs,
        vlm: case.vlm,
        picture_description_preset: case.preset,
        ocr_preset: "rapidocr",
        do_ocr: true,
        do_picture_description: case.picture,
        do_code_enrichment: case.code,
        do_formula_enrichment: case.formula,
    }
}

struct EnrichmentCase<'a> {
    input_format: InputFormat,
    vlm: Option<&'a VlmSettings>,
    preset: Option<&'a str>,
    picture: bool,
    code: bool,
    formula: bool,
}

/// Both enrichment toggles off: the booleans are still sent explicitly, and no
/// unnecessary code/formula custom configuration is attached even when a full
/// legacy bundle is stored.
#[test]
fn disabled_enrichment_still_sends_booleans_without_custom_configs() {
    let bytes = Bytes::from_static(b"x");
    let outputs = [OutputFormat::Md];
    let vlm = vlm_settings();
    let request = enrichment_request(
        &bytes,
        &outputs,
        EnrichmentCase {
            input_format: InputFormat::Pdf,
            vlm: Some(&vlm),
            preset: None,
            picture: false,
            code: false,
            formula: false,
        },
    );

    let fields = enrichment_form_fields(&request)
        .expect("the form fields resolve")
        .expect("a pdf carries the enrichment fields");
    assert!(fields.do_ocr);
    assert!(!fields.do_picture_description);
    assert!(!fields.do_code_enrichment);
    assert!(!fields.do_formula_enrichment);
    assert!(fields.picture_description_preset.is_none());
    assert!(fields.vlm_pipeline_custom_config.is_none());
    assert!(fields.picture_description_custom_config.is_none());
    assert!(fields.code_formula_custom_config.is_none());
}

/// Code enrichment without picture description uses the complete legacy code
/// configuration, while every picture branch stays absent.
#[test]
fn code_enrichment_alone_uses_the_legacy_code_config() {
    let bytes = Bytes::from_static(b"x");
    let outputs = [OutputFormat::Md];
    let vlm = vlm_settings();
    let request = enrichment_request(
        &bytes,
        &outputs,
        EnrichmentCase {
            input_format: InputFormat::Pdf,
            vlm: Some(&vlm),
            preset: None,
            picture: false,
            code: true,
            formula: false,
        },
    );

    let fields = enrichment_form_fields(&request)
        .expect("the form fields resolve")
        .expect("a pdf carries the enrichment fields");
    assert!(fields.do_code_enrichment);
    assert!(!fields.do_formula_enrichment);
    assert!(fields.vlm_pipeline_custom_config.is_none());
    assert!(fields.picture_description_custom_config.is_none());
    let code_formula: serde_json::Value = serde_json::from_str(
        fields
            .code_formula_custom_config
            .as_deref()
            .expect("the code configuration is attached"),
    )
    .expect("the code configuration is JSON");
    assert_eq!(code_formula["extract_code"], serde_json::json!(true));
    assert_eq!(code_formula["extract_formulas"], serde_json::json!(false));
}

/// Formula enrichment alone reflects its own toggle and leaves code extraction
/// disabled instead of silently enabling it.
#[test]
fn formula_enrichment_alone_disables_the_code_extraction() {
    let bytes = Bytes::from_static(b"x");
    let outputs = [OutputFormat::Md];
    let vlm = vlm_settings();
    let request = enrichment_request(
        &bytes,
        &outputs,
        EnrichmentCase {
            input_format: InputFormat::Docx,
            vlm: Some(&vlm),
            preset: None,
            picture: false,
            code: false,
            formula: true,
        },
    );

    let fields = enrichment_form_fields(&request)
        .expect("the form fields resolve")
        .expect("a docx carries the enrichment fields");
    assert!(!fields.do_code_enrichment);
    assert!(fields.do_formula_enrichment);
    let code_formula: serde_json::Value = serde_json::from_str(
        fields
            .code_formula_custom_config
            .as_deref()
            .expect("the formula configuration is attached"),
    )
    .expect("the formula configuration is JSON");
    assert_eq!(code_formula["extract_code"], serde_json::json!(false));
    assert_eq!(code_formula["extract_formulas"], serde_json::json!(true));
}

/// Picture description with code and formula off keeps its own config and never
/// attaches the code/formula configuration.
#[test]
fn picture_description_alone_attaches_only_its_own_config() {
    let bytes = Bytes::from_static(b"x");
    let outputs = [OutputFormat::Md];
    let vlm = vlm_settings();
    let request = enrichment_request(
        &bytes,
        &outputs,
        EnrichmentCase {
            input_format: InputFormat::Pdf,
            vlm: Some(&vlm),
            preset: None,
            picture: true,
            code: false,
            formula: false,
        },
    );

    let fields = enrichment_form_fields(&request)
        .expect("the form fields resolve")
        .expect("a pdf carries the enrichment fields");
    assert!(fields.do_picture_description);
    assert!(fields.vlm_pipeline_custom_config.is_some());
    assert!(fields.picture_description_custom_config.is_some());
    assert!(fields.code_formula_custom_config.is_none());
}

/// A selected picture preset suppresses only the picture-description custom
/// configuration; the pipeline stays available.
#[test]
fn a_picture_preset_suppresses_only_the_picture_custom_config() {
    let bytes = Bytes::from_static(b"x");
    let outputs = [OutputFormat::Md];
    let vlm = vlm_settings();
    let request = enrichment_request(
        &bytes,
        &outputs,
        EnrichmentCase {
            input_format: InputFormat::Pdf,
            vlm: Some(&vlm),
            preset: Some("granite_vision"),
            picture: true,
            code: false,
            formula: false,
        },
    );

    let fields = enrichment_form_fields(&request)
        .expect("the form fields resolve")
        .expect("a pdf carries the enrichment fields");
    assert_eq!(
        fields.picture_description_preset.as_deref(),
        Some("granite_vision")
    );
    assert!(fields.picture_description_custom_config.is_none());
    assert!(fields.vlm_pipeline_custom_config.is_some());
}

/// Without a complete legacy bundle the booleans still reach the remote, which
/// then uses its own server-side model and never needs a local credential.
#[test]
fn enrichment_without_a_legacy_bundle_relies_on_the_server_default() {
    let bytes = Bytes::from_static(b"x");
    let outputs = [OutputFormat::Md];
    let request = enrichment_request(
        &bytes,
        &outputs,
        EnrichmentCase {
            input_format: InputFormat::Pdf,
            vlm: None,
            preset: None,
            picture: false,
            code: true,
            formula: true,
        },
    );

    let fields = enrichment_form_fields(&request)
        .expect("the form fields resolve")
        .expect("a pdf carries the enrichment fields");
    assert!(fields.do_code_enrichment);
    assert!(fields.do_formula_enrichment);
    assert!(fields.vlm_pipeline_custom_config.is_none());
    assert!(fields.picture_description_custom_config.is_none());
    assert!(fields.code_formula_custom_config.is_none());
}

/// An unsupported input format carries none of the enrichment fields.
#[test]
fn an_unsupported_input_carries_no_enrichment_fields() {
    let bytes = Bytes::from_static(b"x");
    let outputs = [OutputFormat::Json];
    let vlm = vlm_settings();
    let request = enrichment_request(
        &bytes,
        &outputs,
        EnrichmentCase {
            input_format: InputFormat::Xlsx,
            vlm: Some(&vlm),
            preset: None,
            picture: true,
            code: true,
            formula: true,
        },
    );

    assert!(
        enrichment_form_fields(&request)
            .expect("the form fields resolve")
            .is_none()
    );
}
