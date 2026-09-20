//! Raw transport contract tests (Redmine 362 Task 4b, sdk-raw).
//!
//! SDK-only: every type under test comes from `context69_sdk` plus test
//! scaffolding (`axum`, `serde_json`). The registry in
//! `context69_sdk::OPERATIONS` must match the current OpenAPI exactly; path,
//! query, auth, idempotency, and response/error handling are verified against
//! a local mock server without touching HTTP/MCP/frontend contracts.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use axum::extract::OriginalUri;
use axum::http::{HeaderMap, Method, StatusCode};
use axum::routing::{delete, get, post};
use context69_sdk::{
    BodyKind, Context69Client, HealthResponse, OPERATIONS, RawRequest, TextBatchRequest,
    find_operation, operation_ids,
};
use serde_json::{Value, json};

const OPENAPI_JSON: &str = include_str!("../../../frontend/openapi/context69.openapi.json");

#[derive(Debug, Clone)]
struct Captured {
    method: String,
    path: String,
    query: Option<String>,
    auth: Option<String>,
    idempotency: Option<String>,
    content_type: Option<String>,
    body: String,
}

type SharedLog = Arc<Mutex<Vec<Captured>>>;

fn record(method: &Method, uri: &OriginalUri, headers: &HeaderMap, body: &str, log: &SharedLog) {
    log.lock().expect("log").push(Captured {
        method: method.to_string(),
        path: uri.path().to_string(),
        query: uri.query().map(str::to_string),
        auth: headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string),
        idempotency: headers
            .get("idempotency-key")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string),
        content_type: headers
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string),
        body: body.to_string(),
    });
}

fn last(log: &SharedLog) -> Captured {
    log.lock().expect("log").last().expect("request").clone()
}

fn authed_client(base_url: &str) -> Context69Client {
    Context69Client::builder()
        .base_url(base_url)
        .expect("base_url")
        .with_personal_access_token("ctx_pat_raw-transport-test-token")
        .expect("pat")
        .build()
        .expect("client")
}

fn unauthed_client(base_url: &str) -> Context69Client {
    Context69Client::builder()
        .base_url(base_url)
        .expect("base_url")
        .build()
        .expect("client")
}

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
fn registry_is_sorted_unique_and_complete() {
    assert_eq!(
        OPERATIONS.len(),
        112,
        "registry must cover all 112 operations"
    );
    let ids = operation_ids();
    assert_eq!(ids.len(), 112);
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    assert_eq!(ids, sorted, "OPERATIONS must be sorted by operation_id");
    let unique: HashSet<_> = ids.iter().collect();
    assert_eq!(unique.len(), 112, "operation ids must be unique");
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
    assert_eq!(openapi.len(), 112, "OpenAPI must expose 112 operations");
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

#[tokio::test]
async fn raw_auth_query_and_idempotency_headers_match_registry() {
    let shared: SharedLog = Arc::new(Mutex::new(Vec::new()));
    let c1 = shared.clone();
    let c2 = shared.clone();
    let c3 = shared.clone();
    let c4 = shared.clone();
    let router = axum::Router::new()
        .route(
            "/healthz",
            get(
                move |method: Method,
                      uri: OriginalUri,
                      headers: HeaderMap| async move {
                    record(&method, &uri, &headers, "", &c1);
                    (StatusCode::OK, axum::Json(json!({"status": "ok"})))
                },
            ),
        )
        .route(
            "/v1/tasks/{task_id}",
            get(
                move |method: Method,
                      uri: OriginalUri,
                      headers: HeaderMap| async move {
                    record(&method, &uri, &headers, "", &c2);
                    (
                        StatusCode::OK,
                        axum::Json(json!({
                            "task_id": "11111111-1111-1111-1111-111111111111",
                            "kind": "text_batch",
                            "status": "succeeded",
                            "origin": "manual",
                            "group_path": "research",
                            "source_key": null,
                            "stage": null,
                            "waiting_reason": null,
                            "dependency_key": null,
                            "progress": {"total": 1, "queued": 0, "running": 0, "waiting": 0, "succeeded": 1, "failed": 0, "cancelled": 0},
                            "failure_stage": null,
                            "error_summary": null,
                            "eta_seconds": null,
                            "created_at": "2026-01-01T00:00:00Z",
                            "started_at": "2026-01-01T00:00:00Z",
                            "finished_at": "2026-01-01T00:00:00Z",
                            "updated_at": "2026-01-01T00:00:00Z",
                            "deleted_at": null
                        })),
                    )
                },
            ),
        )
        .route(
            "/v1/groups/by-path/{group_path}/batch/text",
            post(
                move |method: Method,
                      uri: OriginalUri,
                      headers: HeaderMap,
                      body: String| async move {
                    record(&method, &uri, &headers, &body, &c3);
                    (
                        StatusCode::ACCEPTED,
                        axum::Json(json!({
                            "task_id": "11111111-1111-1111-1111-111111111111",
                            "item_ids": []
                        })),
                    )
                },
            ),
        )
        .route(
            "/v1/auth/logout",
            post(
                move |method: Method,
                      uri: OriginalUri,
                      headers: HeaderMap| async move {
                    record(&method, &uri, &headers, "", &c4);
                    StatusCode::NO_CONTENT
                },
            ),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, router).await.expect("serve");
    });
    let base_url = format!("http://{addr}");
    let authed = authed_client(&base_url);
    let unauthed = unauthed_client(&base_url);

    // Public operation needs no token and sends no Authorization.
    let health = RawRequest::new("healthz").expect("healthz");
    assert!(!health.operation().requires_auth);
    unauthed
        .raw()
        .execute_empty(&health)
        .await
        .expect("unauthenticated healthz");
    assert_eq!(last(&shared).auth, None);
    assert_eq!(last(&shared).path, "/healthz");
    authed
        .raw()
        .execute_empty(&health)
        .await
        .expect("authed healthz");
    assert_eq!(
        last(&shared).auth,
        None,
        "public ops never send Authorization"
    );
    assert_eq!(last(&shared).path, "/healthz");

    // Protected operation without a token fails before any HTTP call.
    let task = RawRequest::new("get_task")
        .expect("get_task")
        .path_param("task_id", "11111111-1111-1111-1111-111111111111");
    assert!(task.operation().requires_auth);
    let before = shared.lock().expect("log").len();
    let error = unauthed
        .raw()
        .execute(&task)
        .await
        .expect_err("must require auth");
    assert!(
        matches!(error, context69_sdk::Error::AuthenticationRequired),
        "unexpected {error:?}"
    );
    assert_eq!(shared.lock().expect("log").len(), before);

    // Protected operation with a token sends Bearer.
    authed
        .raw()
        .execute_empty(&RawRequest::new("logout").expect("logout"))
        .await
        .expect("logout");
    assert_eq!(last(&shared).auth, None, "logout is public per registry");
    let task_response = authed.raw().execute(&task).await.expect("get_task");
    assert_eq!(task_response.status, StatusCode::OK);
    assert_eq!(
        last(&shared).auth.as_deref(),
        Some("Bearer ctx_pat_raw-transport-test-token")
    );
    assert_eq!(last(&shared).method, "GET");
    assert_eq!(
        last(&shared).path,
        "/v1/tasks/11111111-1111-1111-1111-111111111111"
    );

    // Idempotency: absent by default, stable when requested.
    let batch_body = TextBatchRequest { items: vec![] };
    let batch = RawRequest::new("submit_text_batch")
        .expect("batch")
        .path_param("group_path", "research")
        .json_body(&batch_body)
        .expect("body");
    assert!(batch.operation().idempotent);
    assert_eq!(batch.idempotency_key_value(), None);
    authed
        .raw()
        .execute(&batch)
        .await
        .expect("batch without key");
    assert_eq!(last(&shared).idempotency, None);
    assert_eq!(last(&shared).path, "/v1/groups/by-path/research/batch/text");
    assert!(
        last(&shared)
            .content_type
            .as_deref()
            .unwrap_or_default()
            .contains("application/json"),
        "json body must set content-type, got {:?}",
        last(&shared).content_type
    );
    assert!(
        last(&shared).body.contains("items"),
        "json body must reach the server, got {}",
        last(&shared).body
    );
    let keyed = RawRequest::new("submit_text_batch")
        .expect("batch")
        .path_param("group_path", "research")
        .json_body(&batch_body)
        .expect("body")
        .with_auto_idempotency_key()
        .expect("auto key");
    let first = keyed.idempotency_key_value().expect("key").to_string();
    assert!(first.starts_with("ctx69-sdk-"));
    let repeated = RawRequest::new("submit_text_batch")
        .expect("batch")
        .path_param("group_path", "research")
        .json_body(&batch_body)
        .expect("body")
        .with_auto_idempotency_key()
        .expect("auto key");
    assert_eq!(repeated.idempotency_key_value(), Some(first.as_str()));
    authed.raw().execute(&keyed).await.expect("batch with key");
    assert_eq!(last(&shared).idempotency.as_deref(), Some(first.as_str()));
    let different = RawRequest::new("submit_text_batch")
        .expect("batch")
        .path_param("group_path", "research")
        .json_value(json!({"items": [{"external_id": "other"}]}))
        .with_auto_idempotency_key()
        .expect("auto key");
    assert_ne!(different.idempotency_key_value(), Some(first.as_str()));
    // Only the six task-submitting operations are marked idempotent.
    let idempotent_ids: HashSet<_> = OPERATIONS
        .iter()
        .filter(|op| op.idempotent)
        .map(|op| op.id)
        .collect();
    let expected: HashSet<_> = [
        "submit_text_batch",
        "submit_url_batch",
        "submit_file_batch",
        "submit_delete_batch",
        "submit_task",
        "submit_vector_index_rebuild",
    ]
    .into_iter()
    .collect();
    assert_eq!(idempotent_ids, expected);
}

#[tokio::test]
async fn raw_query_encoding_response_and_error_decoding() {
    let shared: SharedLog = Arc::new(Mutex::new(Vec::new()));
    let c1 = shared.clone();
    let router = axum::Router::new()
        .route(
            "/v1/groups/by-path/{group_path}/members",
            get(
                move |method: Method,
                      uri: OriginalUri,
                      headers: HeaderMap| async move {
                    record(&method, &uri, &headers, "", &c1);
                    (
                        StatusCode::OK,
                        axum::Json(json!({"items": [], "pagination": {"page": 1, "page_size": 25, "total": 0, "total_pages": 0}})),
                    )
                },
            ),
        )
        .route(
            "/healthz",
            get(
                move || async {
                    (
                        StatusCode::OK,
                        axum::Json(json!({"status": "ok"})),
                    )
                },
            ),
        )
        .route(
            "/v1/tasks/{task_id}",
            delete(
                move || async { (StatusCode::NOT_FOUND, axum::Json(json!({"code": "not_found", "message": "task missing"}))) },
            ),
        )
        .route(
            "/v1/auth/personal-access-tokens/{token_id}",
            delete(move || async { StatusCode::NO_CONTENT }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, router).await.expect("serve");
    });
    let client = authed_client(&format!("http://{addr}"));

    // Query pairs with spaces and reserved chars round-trip through the mock.
    let members = RawRequest::new("list_group_members")
        .expect("members")
        .path_param("group_path", "research")
        .query("query", "hello world &=")
        .query("page", "1")
        .query_opt::<String>("sort_by", None);
    let response = client.raw().execute(&members).await.expect("members");
    assert_eq!(response.status, StatusCode::OK);
    let captured = last(&shared);
    assert_eq!(captured.path, "/v1/groups/by-path/research/members");
    assert_eq!(captured.method, "GET");
    let raw_query = captured.query.expect("query sent");
    assert!(
        raw_query.contains("query="),
        "query key missing: {raw_query}"
    );
    assert!(
        !raw_query.contains("sort_by="),
        "omitted query must be absent: {raw_query}"
    );
    assert!(
        captured.body.is_empty(),
        "GET must send no body, got {}",
        captured.body
    );

    // Success JSON decodes into the shared contract type from the SDK alone.
    let health: HealthResponse = client
        .raw()
        .execute_decode(&RawRequest::new("healthz").expect("healthz"))
        .await
        .expect("decode health");
    assert_eq!(
        serde_json::to_value(&health)
            .expect("serialize")
            .get("status")
            .and_then(|v| v.as_str()),
        Some("ok")
    );

    // Error JSON maps to HttpStatus with the parsed api_error message.
    let missing = RawRequest::new("delete_task")
        .expect("delete")
        .path_param("task_id", "11111111-1111-1111-1111-111111111111");
    let error = client
        .raw()
        .execute(&missing)
        .await
        .expect("transport ok")
        .decode_empty()
        .expect_err("404 must error");
    match error {
        context69_sdk::Error::HttpStatus {
            status,
            api_error,
            body,
        } => {
            assert_eq!(status, StatusCode::NOT_FOUND);
            assert_eq!(api_error.as_deref(), Some("task missing"));
            assert!(body.contains("task missing"));
        }
        other => panic!("unexpected error {other:?}"),
    }

    // 204 with an empty body is accepted without decoding JSON.
    let revoke = RawRequest::new("revoke_personal_access_token")
        .expect("revoke")
        .path_param("token_id", "11111111-1111-1111-1111-111111111111");
    client
        .raw()
        .execute_empty(&revoke)
        .await
        .expect("204 empty");

    // Decoding an empty body as JSON fails honestly instead of returning default.
    let empty_ok = client.raw().execute(&revoke).await.expect("revoke raw");
    assert!(empty_ok.decode::<HealthResponse>().is_err());
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
            rendered.starts_with("/healthz") || rendered.starts_with("/v1/"),
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
