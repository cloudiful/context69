//! The wire models of the Docling protocol.

use crate::docling::models::{
    ConversionStatus, InputFormat, OutputFormat, ParsedResult, VlmSettings, code_formula_options,
    parse_result, parse_task_list, parse_task_status, picture_description_options,
    vlm_pipeline_options,
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
fn the_known_terminal_states_are_exactly_the_documented_values() {
    for terminal in [
        "success",
        "partial_success",
        "failure",
        "skipped",
        "cancelled",
    ] {
        let status = ConversionStatus::parse(terminal).expect("known state");
        assert!(status.is_terminal(), "{terminal} is terminal");
        assert_eq!(status.wire_name(), terminal);
    }
    for nonterminal in ["pending", "started"] {
        let status = ConversionStatus::parse(nonterminal).expect("known state");
        assert!(!status.is_terminal(), "{nonterminal} is not terminal");
    }
}

#[test]
fn a_successful_state_is_success_or_partial_success() {
    assert!(
        ConversionStatus::parse("success")
            .expect("state")
            .is_successful()
    );
    assert!(
        ConversionStatus::parse("partial_success")
            .expect("state")
            .is_successful()
    );
    for terminal in ["failure", "skipped", "cancelled"] {
        let status = ConversionStatus::parse(terminal).expect("state");
        assert!(status.is_terminal());
        assert!(!status.is_successful(), "{terminal} carries no result");
    }
}

#[test]
fn an_unknown_future_state_stays_readable_and_non_terminal() {
    let status = ConversionStatus::parse("mystery_future").expect("unknown state");
    assert_eq!(status.wire_name(), "mystery_future");
    assert!(!status.is_terminal());
    assert!(!status.is_successful());
}

#[test]
fn an_empty_state_is_malformed() {
    assert!(ConversionStatus::parse("").is_err());
}

#[test]
fn the_wire_spelling_is_used_instead_of_a_debug_rendering() {
    assert_eq!(
        ConversionStatus::parse("partial_success")
            .expect("state")
            .wire_name(),
        "partial_success"
    );
    assert_ne!(
        ConversionStatus::parse("partial_success")
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
            "category": "content",
            "message": "page 3 failed",
            "retryable": false,
            "phase": "conversion"
        }
    }"#;
    let status = parse_task_status(body).expect("status");
    assert_eq!(status.task_id, "task-1");
    assert_eq!(status.task_status, ConversionStatus::Failure);
    assert_eq!(status.failure_message(), Some("outer"));
}

#[test]
fn a_task_status_response_without_a_state_is_malformed() {
    let body = br#"{"task_id": "task-1"}"#;
    assert!(parse_task_status(body).is_err());
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

#[test]
fn a_failed_task_result_is_a_structured_failure() {
    let body = br#"{
        "kind": "TaskFailureResult",
        "failure": {
            "category": "content",
            "message": "the document could not be converted",
            "retryable": false,
            "phase": "conversion"
        }
    }"#;
    match parse_result(body).expect("result") {
        ParsedResult::Failed { status, message } => {
            assert_eq!(status, "content");
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
    let options = code_formula_options(&vlm_settings());
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
        format!("{:?}", code_formula_options(&vlm)),
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

    let code_formula = serde_json::to_value(code_formula_options(&vlm)).expect("json");
    assert_eq!(
        code_formula["engine_options"]["headers"]["Authorization"],
        "Bearer provider-secret"
    );
    assert_eq!(code_formula["engine_options"]["timeout"], 30);
}
