//! `get_code` MCP contract tests (issue #681 phase 5I).
//!
//! Pure contract checks with no database and no provider: the tool input
//! deserializes with truthful schema bounds and defaults, unsafe paths and line
//! windows are refused without echoing input, the response exposes exactly the
//! bounded provenance/text fields, and no blob, provider, secret, or connection
//! field can appear.

use context69::contracts::sources::GIT_REPOSITORY_FILE_PATH_MAX_CHARS;
use context69::contracts::{
    MCP_CODE_GET_CHUNK_LIMIT_DEFAULT, MCP_CODE_GET_CHUNK_LIMIT_MAX, McpCodeCoverage, McpCodeFile,
    McpCodeGeneration, McpGetCodeRequest, McpGetCodeResponse,
};
use serde_json::{Value, json};
use uuid::Uuid;

fn schema_of<T: schemars::JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T)).expect("schema serializes")
}

fn prop(schema: &Value, name: &str) -> Value {
    schema
        .get("properties")
        .and_then(|properties| properties.get(name))
        .cloned()
        .unwrap_or_else(|| panic!("schema has no property {name}: {schema}"))
}

fn keyword(schema: &Value, prop_name: &str, keyword: &str) -> Value {
    prop(schema, prop_name)
        .get(keyword)
        .cloned()
        .unwrap_or_else(|| panic!("property {prop_name} has no {keyword}: {schema}"))
}

fn prop_names(schema: &Value) -> Vec<String> {
    schema
        .get("properties")
        .and_then(Value::as_object)
        .map(|map| {
            let mut names: Vec<String> = map.keys().cloned().collect();
            names.sort();
            names
        })
        .unwrap_or_default()
}

fn generation() -> McpCodeGeneration {
    McpCodeGeneration::new(
        Uuid::new_v4(),
        Uuid::new_v4(),
        7,
        "refs/heads/main",
        &"b".repeat(40),
        None,
        McpCodeCoverage {
            file_count: 12,
            excluded_file_count: 3,
            total_bytes: 4_096,
        },
    )
}

#[test]
fn request_schema_declares_truthful_bounds_and_default() {
    let schema = schema_of::<McpGetCodeRequest>();
    assert_eq!(keyword(&schema, "group_path", "maxLength"), json!(1024));
    assert_eq!(
        keyword(&schema, "path", "maxLength"),
        json!(GIT_REPOSITORY_FILE_PATH_MAX_CHARS)
    );
    assert_eq!(keyword(&schema, "path", "minLength"), json!(1));
    assert_eq!(keyword(&schema, "start_line", "minimum"), json!(1));
    assert_eq!(keyword(&schema, "end_line", "minimum"), json!(1));
    assert_eq!(keyword(&schema, "chunk_limit", "minimum"), json!(1));
    assert_eq!(
        keyword(&schema, "chunk_limit", "maximum"),
        json!(MCP_CODE_GET_CHUNK_LIMIT_MAX)
    );
    assert_eq!(
        keyword(&schema, "chunk_limit", "default"),
        json!(MCP_CODE_GET_CHUNK_LIMIT_DEFAULT)
    );

    let required = schema
        .get("required")
        .and_then(Value::as_array)
        .expect("request schema declares required fields");
    for field in [
        "group_path",
        "repository_key",
        "path",
        "start_line",
        "end_line",
    ] {
        assert!(
            required.iter().any(|value| value == field),
            "request schema must require {field}: {schema}"
        );
    }
    assert!(
        !required.iter().any(|value| value == "chunk_limit"),
        "chunk_limit must stay optional: {schema}"
    );
}

#[test]
fn response_schema_exposes_exactly_the_bounded_window_fields() {
    let schema = schema_of::<McpGetCodeResponse>();
    assert_eq!(
        prop_names(&schema),
        vec![
            "byte_count",
            "end_line",
            "file",
            "generation",
            "start_line",
            "text",
            "truncated",
        ],
        "response must not carry HTTP/blob/provider fields: {schema}"
    );
    assert_eq!(keyword(&schema, "text", "maxLength"), json!(65536));
    assert_eq!(keyword(&schema, "truncated", "type"), json!("boolean"));

    let file = schema_of::<McpCodeFile>();
    assert_eq!(
        prop_names(&file),
        vec!["file_key", "language", "path"],
        "the manifest projection carries no provider blob id: {file}"
    );
    assert_eq!(keyword(&file, "path", "maxLength"), json!(512));
    assert_eq!(keyword(&file, "language", "maxLength"), json!(32));

    // No provider, blob, secret, connection, or HTTP-content field survives.
    for forbidden in [
        "provider_blob_sha",
        "provider_blob_id",
        "blob",
        "blob_content",
        "connection",
        "credential",
        "secret",
        "secret_reference",
        "index_status",
        "checkpoint",
        "pagination",
        "next_cursor",
        "has_more",
    ] {
        assert!(
            schema
                .get("properties")
                .and_then(|properties| properties.get(forbidden))
                .is_none(),
            "forbidden field {forbidden} leaked into the get_code response"
        );
    }
}

#[test]
fn request_deserializes_with_defaults_and_rejects_unknown_fields() {
    let repository_key = Uuid::new_v4();
    let minimal: McpGetCodeRequest = serde_json::from_value(json!({
        "group_path": "research/context69",
        "repository_key": repository_key,
        "path": "src/mcp/tools/code.rs",
        "start_line": 10,
        "end_line": 20,
    }))
    .expect("minimal payload deserializes");
    assert_eq!(minimal.chunk_limit, MCP_CODE_GET_CHUNK_LIMIT_DEFAULT);
    assert!(minimal.validate().is_ok());

    let full: McpGetCodeRequest = serde_json::from_value(json!({
        "group_path": "research/context69",
        "repository_key": repository_key,
        "path": "src/mcp/tools/code.rs",
        "start_line": 10,
        "end_line": 20,
        "chunk_limit": 8,
    }))
    .expect("full payload deserializes");
    assert_eq!(full.chunk_limit, 8);
    assert!(full.validate().is_ok());

    for extra in [
        json!({"cursor": "next"}),
        json!({"page": 2}),
        json!({"limit": 10}),
        json!({"generation_key": Uuid::new_v4()}),
    ] {
        let mut payload = json!({
            "group_path": "research/context69",
            "repository_key": repository_key,
            "path": "src/mcp/tools/code.rs",
            "start_line": 1,
            "end_line": 2,
        });
        for (key, value) in extra.as_object().expect("object") {
            payload[key] = value.clone();
        }
        assert!(
            serde_json::from_value::<McpGetCodeRequest>(payload).is_err(),
            "HTTP-only/unknown fields must be rejected, not silently ignored"
        );
    }
}

#[test]
fn request_validation_rejects_blank_oversized_and_unsafe_values() {
    let repository_key = Uuid::new_v4();
    let base = json!({
        "group_path": "research/context69",
        "repository_key": repository_key,
        "path": "src/mcp/tools/code.rs",
        "start_line": 1,
        "end_line": 20,
    });

    let cases: Vec<(Value, &str)> = vec![
        (json!({"group_path": "  "}), "blank group_path"),
        (json!({"path": ""}), "empty path"),
        (json!({"path": "  "}), "blank path"),
        (
            json!({"path": "x".repeat(GIT_REPOSITORY_FILE_PATH_MAX_CHARS + 1)}),
            "oversized path",
        ),
        (json!({"path": "/absolute"}), "absolute path"),
        (json!({"path": "src/../secret"}), "traversing path"),
        (json!({"path": "src/./lib"}), "dot segment"),
        (json!({"path": "src//lib"}), "empty segment"),
        (json!({"path": "src\\lib"}), "backslash segment"),
        (json!({"path": "src/\u{1}"}), "control character path"),
        (json!({"start_line": 0}), "zero start_line"),
        (json!({"end_line": 0}), "zero end_line"),
        (json!({"start_line": 30, "end_line": 10}), "reversed window"),
        (
            json!({"start_line": 1, "end_line": 401}),
            "window wider than 400 lines",
        ),
        (json!({"chunk_limit": 0}), "chunk_limit below minimum"),
        (
            json!({"chunk_limit": MCP_CODE_GET_CHUNK_LIMIT_MAX + 1}),
            "chunk_limit above maximum",
        ),
    ];

    for (patch, label) in cases {
        let mut payload = base.clone();
        for (key, value) in patch.as_object().expect("object") {
            payload[key] = value.clone();
        }
        let request: McpGetCodeRequest =
            serde_json::from_value(payload.clone()).expect("payload shape deserializes");
        assert!(
            request.validate().is_err(),
            "must reject {label}: {payload}"
        );
    }

    let valid: McpGetCodeRequest = serde_json::from_value(json!({
        "group_path": "research/context69",
        "repository_key": repository_key,
        "path": ".github/workflows/ci.yml",
        "start_line": 1,
        "end_line": 400,
        "chunk_limit": MCP_CODE_GET_CHUNK_LIMIT_MAX,
    }))
    .expect("valid payload deserializes");
    assert!(valid.validate().is_ok());
}

#[test]
fn response_roundtrips_verbatim_text_and_exact_byte_count() {
    let text = "let x = SafeRef::parse(&raw)?;\r\n  // 100% café\n";
    let response = McpGetCodeResponse {
        generation: generation(),
        file: McpCodeFile {
            file_key: Uuid::new_v4(),
            path: "src/mcp/tools/code_get.rs".to_string(),
            language: "rust".to_string(),
        },
        start_line: 10,
        end_line: 11,
        text: text.to_string(),
        byte_count: text.len() as i64,
        truncated: false,
    };

    let value = serde_json::to_value(&response).expect("response serializes");
    assert_eq!(value["text"], json!(text));
    assert_eq!(value["byte_count"], json!(text.len() as i64));
    assert_eq!(value["truncated"], json!(false));
    assert_eq!(value["start_line"], json!(10));
    assert_eq!(value["end_line"], json!(11));
    assert_eq!(value["file"]["language"], json!("rust"));
    assert_eq!(value["generation"]["generation_number"], json!(7));
    assert_eq!(value["generation"]["coverage"]["file_count"], json!(12));
}
