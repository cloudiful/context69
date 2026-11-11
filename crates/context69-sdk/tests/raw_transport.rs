//! Raw transport contract tests (Redmine 362 Task 4b, sdk-raw).
//!
//! SDK-only raw transport contract tests.
//!
//! The registry in `context69_sdk::OPERATIONS` must match the current OpenAPI
//! exactly; path, query, body-shape, and facade-type contracts are asserted
//! without any server or router.

use std::collections::HashSet;

use context69_sdk::{
    BodyKind, HealthResponse, OPERATIONS, RawRequest, TextBatchRequest, find_operation,
    operation_ids,
};
use serde_json::{Value, json};

const OPENAPI_JSON: &str = include_str!("../../../frontend/openapi/context69.openapi.json");

fn openapi_operations() -> Vec<(String, String, String)> {
    let document: Value = serde_json::from_str(OPENAPI_JSON).expect("openapi parses");
    let paths = document
        .get("paths")
        .and_then(|v| v.as_object())
        .expect("paths exist");
    let mut out = Vec::new();
    for (path, methods) in paths {
        let methods = methods.as_object().expect("path object");
        for method in ["get", "post", "put", "patch", "delete"] {
            if let Some(op) = methods.get(method) {
                let oid = op
                    .get("operationId")
                    .and_then(|v| v.as_str())
                    .expect("operationId")
                    .to_string();
                out.push((oid, method.to_uppercase(), path.clone()));
            }
        }
    }
    out.sort();
    out
}

fn openapi_entry(operation_id: &str) -> Value {
    let document: Value = serde_json::from_str(OPENAPI_JSON).expect("openapi parses");
    let paths = document
        .get("paths")
        .and_then(|v| v.as_object())
        .expect("paths exist");
    for methods in paths.values() {
        let methods = methods.as_object().expect("path object");
        for op in methods.values() {
            if op.get("operationId").and_then(|v| v.as_str()) == Some(operation_id) {
                return op.clone();
            }
        }
    }
    panic!("operation {operation_id} missing in OpenAPI");
}

fn expected_body_kind(op: &Value) -> BodyKind {
    match op.get("requestBody") {
        None => BodyKind::Empty,
        Some(body) => {
            let content = body
                .get("content")
                .and_then(|v| v.as_object())
                .expect("content");
            if content.contains_key("application/json") {
                BodyKind::Json
            } else if content.contains_key("multipart/form-data") {
                BodyKind::Multipart
            } else {
                panic!("unexpected body content for {op:?}");
            }
        }
    }
}

#[test]
fn health_probe_operations_are_registered_as_public_paths() {
    for (id, path) in [("livez", "/livez"), ("readyz", "/readyz")] {
        let operation = find_operation(id).unwrap_or_else(|| panic!("{id} must be registered"));
        assert_eq!(operation.method, "GET", "{id} method");
        assert_eq!(operation.path_template, path, "{id} path");
        assert!(!operation.requires_auth, "{id} must stay public");
        assert_eq!(operation.response_schema, "HealthResponse", "{id} response");
        assert_eq!(operation.success_status, 200, "{id} success status");
    }
}

#[test]
fn registry_is_sorted_unique_and_complete() {
    assert_eq!(
        OPERATIONS.len(),
        152,
        "registry must cover all 152 operations"
    );
    let ids = operation_ids();
    assert_eq!(ids.len(), 152);
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    assert_eq!(ids, sorted, "OPERATIONS must be sorted by operation_id");
    let unique: HashSet<_> = ids.iter().collect();
    assert_eq!(unique.len(), 152, "operation ids must be unique");
    for op in OPERATIONS {
        assert!(!op.id.is_empty());
        assert!(["GET", "POST", "PUT", "PATCH", "DELETE"].contains(&op.method));
        assert!(op.path_template.starts_with('/'));
        assert!(op.success_status >= 200 && op.success_status < 300);
        assert!(find_operation(op.id).is_some());
    }
    assert!(find_operation("no_such_operation").is_none());
    assert!(RawRequest::new("no_such_operation").is_err());
}

#[test]
fn registry_matches_openapi_exactly_with_no_omission() {
    let openapi = openapi_operations();
    assert_eq!(openapi.len(), 152, "OpenAPI must expose 152 operations");
    let registry: Vec<(String, String, String)> = OPERATIONS
        .iter()
        .map(|op| {
            (
                op.id.to_string(),
                op.method.to_string(),
                op.path_template.to_string(),
            )
        })
        .collect();
    let openapi_set: HashSet<_> = openapi.iter().collect();
    let registry_set: HashSet<_> = registry.iter().collect();
    assert_eq!(
        openapi_set, registry_set,
        "registry (method/path/operation_id) must equal OpenAPI exactly"
    );
    for (oid, method, path) in &openapi {
        let reg = find_operation(oid).unwrap_or_else(|| panic!("registry omits {oid}"));
        assert_eq!(reg.method, method, "method mismatch for {oid}");
        assert_eq!(reg.path_template, path, "path mismatch for {oid}");
        let entry = openapi_entry(oid);
        assert_eq!(
            reg.body_kind,
            expected_body_kind(&entry),
            "body kind mismatch for {oid}"
        );
    }
    for op in OPERATIONS {
        let entry = openapi_entry(op.id);
        let responses = entry
            .get("responses")
            .and_then(|v| v.as_object())
            .expect("responses");
        let mut success: Vec<u16> = responses
            .keys()
            .filter_map(|code| code.parse::<u16>().ok())
            .filter(|code| (200..300).contains(code))
            .collect();
        success.sort_unstable();
        assert!(!success.is_empty(), "no 2xx for {}", op.id);
        assert_eq!(
            op.success_status, success[0],
            "success status mismatch for {}",
            op.id
        );
    }
}

#[test]
fn registry_request_response_names_match_openapi_refs() {
    let document: Value = serde_json::from_str(OPENAPI_JSON).expect("openapi parses");
    for op in OPERATIONS {
        let entry = openapi_entry(op.id);
        let expected_request = match entry.get("requestBody") {
            None => "-".to_string(),
            Some(body) => {
                let content = body
                    .get("content")
                    .and_then(|v| v.as_object())
                    .expect("content");
                if let Some(json) = content.get("application/json") {
                    let schema = json.get("schema").expect("schema");
                    schema
                        .get("$ref")
                        .and_then(|v| v.as_str())
                        .map(|r| r.rsplit('/').next().expect("ref").to_string())
                        .unwrap_or_else(|| "inline-json".to_string())
                } else if content.contains_key("multipart/form-data") {
                    "multipart-form".to_string()
                } else {
                    panic!("unexpected body for {}", op.id);
                }
            }
        };
        assert_eq!(
            op.request_schema, expected_request,
            "request schema mismatch for {}",
            op.id
        );
        let responses = entry
            .get("responses")
            .and_then(|v| v.as_object())
            .expect("responses");
        let code = op.success_status.to_string();
        let success = responses.get(&code).expect("success response");
        let expected_response = match success.get("content") {
            None => "-".to_string(),
            Some(content) => match content.get("application/json") {
                None => "-".to_string(),
                Some(json) => {
                    let schema = json.get("schema").expect("schema");
                    if let Some(r) = schema.get("$ref").and_then(|v| v.as_str()) {
                        r.rsplit('/').next().expect("ref").to_string()
                    } else if schema.get("type").and_then(|v| v.as_str()) == Some("array") {
                        let items = schema.get("items").expect("items");
                        match items.get("$ref").and_then(|v| v.as_str()) {
                            Some(r) => {
                                format!("array<{}>", r.rsplit('/').next().expect("ref"))
                            }
                            None => "array<inline>".to_string(),
                        }
                    } else {
                        "inline".to_string()
                    }
                }
            },
        };
        assert_eq!(
            op.response_schema, expected_response,
            "response schema mismatch for {}",
            op.id
        );
    }
    // Shared contract names resolve: spot-check the core wire types.
    let schemas = document
        .pointer("/components/schemas")
        .and_then(|v| v.as_object())
        .expect("schemas exist");
    for name in [
        "HealthResponse",
        "SearchRequest",
        "TaskPageResponse",
        "SourceStatus",
        "DocumentResponse",
    ] {
        assert!(schemas.contains_key(name), "OpenAPI must contain {name}");
    }
}

#[test]
fn raw_request_path_query_helpers_encode_explicitly() {
    let request = RawRequest::new("get_group")
        .expect("known op")
        .path_param("group_path", "research/news team")
        .query("verbose", "true")
        .query_opt("missing", None::<String>)
        .query_opt("page", Some(2));
    assert_eq!(request.operation().id, "get_group");
    assert_eq!(request.query_params().len(), 2);
    let rendered = request.render_path().expect("render");
    assert!(
        rendered.contains("research%2Fnews+team") || rendered.contains("research%2Fnews%20team"),
        "group slash must be encoded, got {rendered}"
    );
    assert!(!rendered.contains("{") && !rendered.contains("}"));

    let missing = RawRequest::new("get_task").expect("known op");
    assert!(
        missing.render_path().is_err(),
        "missing task_id must fail instead of sending a templated path"
    );
    let unknown_param = RawRequest::new("healthz")
        .expect("known op")
        .path_param("bogus", "x");
    assert!(unknown_param.render_path().is_err());

    let query_only = RawRequest::new("list_tasks")
        .expect("known op")
        .query("view", "processing")
        .query_opt::<String>("query", None)
        .query("page", "1");
    assert_eq!(query_only.query_params().len(), 2);
    assert_eq!(
        query_only.render_path().expect("no path params"),
        "/v1/tasks"
    );

    let contract_body = TextBatchRequest { items: vec![] };
    let with_body = RawRequest::new("submit_text_batch")
        .expect("known op")
        .path_param("group_path", "research")
        .json_body(&contract_body)
        .expect("shared contract body");
    assert_eq!(with_body.operation().request_schema, "TextBatchRequest");
    let rendered_batch = with_body.render_path().expect("render");
    assert_eq!(rendered_batch, "/v1/groups/by-path/research/batch/text");
}

#[test]
fn raw_multipart_and_stream_operations_are_registered_with_limitation() {
    for id in ["upload_library_files", "upload_group_library_files"] {
        let op = find_operation(id).expect("multipart op registered");
        assert_eq!(op.body_kind, BodyKind::Multipart);
        assert_eq!(op.request_schema, "multipart-form");
        let request = RawRequest::new(id).expect("request").raw_body(
            "multipart/form-data; boundary=test",
            b"pre-encoded".to_vec(),
        );
        assert_eq!(
            request.operation().id,
            id,
            "multipart callers supply pre-encoded bytes"
        );
    }
    for id in ["search_stream", "stream_tasks"] {
        let stream = find_operation(id).expect("stream registered");
        assert_eq!(stream.method, "GET");
        assert_eq!(stream.response_schema, "-");
    }
    // No per-operation typed multipart builder exists: the registry plus
    // generic RawRequest/RawResponse is the complete surface.
    assert_eq!(
        OPERATIONS
            .iter()
            .filter(|op| op.body_kind == BodyKind::Multipart)
            .count(),
        2
    );
}

#[test]
fn every_operation_builds_a_request_without_handwritten_methods() {
    // No operation is omitted from the generic builder: each id constructs,
    // each path template renders with dummy params, and login keeps its
    // shared-contract JSON body name.
    assert_eq!(
        find_operation("login").expect("login").request_schema,
        "AuthLoginRequest"
    );
    for op in OPERATIONS {
        let mut request = RawRequest::new(op.id).unwrap_or_else(|_| panic!("new {}", op.id));
        for placeholder in placeholders(op.path_template) {
            request = request.path_param(&placeholder, dummy_path_value(&placeholder));
        }
        let rendered = request
            .render_path()
            .unwrap_or_else(|_| panic!("render {}", op.id));
        assert!(
            !rendered.contains('{') && !rendered.contains('}'),
            "unrendered template for {}: {rendered}",
            op.id
        );
        assert!(
            ["/healthz", "/livez", "/readyz"]
                .iter()
                .any(|prefix| rendered.starts_with(prefix))
                || rendered.starts_with("/v1/"),
            "unexpected path for {}: {rendered}",
            op.id
        );
    }
}

fn placeholders(template: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        let after = &rest[start + 1..];
        if let Some(end) = after.find('}') {
            out.push(after[..end].to_string());
            rest = &after[end + 1..];
        } else {
            break;
        }
    }
    out
}

fn dummy_path_value(name: &str) -> String {
    match name {
        "task_id" | "file_id" | "folder_id" | "token_id" => {
            "11111111-1111-1111-1111-111111111111".to_string()
        }
        "document_id" => "42".to_string(),
        "index_id" => "7".to_string(),
        _ => "research".to_string(),
    }
}

#[test]
fn facade_types_still_come_from_sdk_alone() {
    // The raw surface reuses shared contracts; it does not duplicate them.
    let body = TextBatchRequest { items: vec![] };
    let value = serde_json::to_value(&body).expect("contract serializes");
    assert_eq!(value, json!({"items": []}));
    let health: HealthResponse =
        serde_json::from_value(json!({"status": "ok"})).expect("contract deserializes");
    assert_eq!(
        serde_json::to_value(&health)
            .expect("serialize")
            .get("status")
            .and_then(|v| v.as_str()),
        Some("ok")
    );
}
