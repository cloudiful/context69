//! SDK facade contract tests (Redmine 362 Task 4a, SDK-only).
//!
//! Only `context69_sdk` (plus test scaffolding crates) is imported here:
//! every request/response type under test must be constructible from the SDK
//! facade alone. Raw 116-operation transport coverage is explicitly Task 4b
//! remaining work and is not claimed by this suite.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::Json;
use axum::extract::OriginalUri;
use axum::http::{HeaderMap, Method, StatusCode};
use axum::routing::{get, post};
use context69_sdk::{
    AuthMeResponse, BatchGetDocumentsRequest, CanonicalSearchRequest, CanonicalTaskListQuery,
    CanonicalUpdateSearchSettingsRequest, CanonicalUploadMetadata, Context69Client, DocumentKey,
    IngestOptions, RebuildDocumentExtractionsRequest, SearchPagination, SearchRequest,
    SearchResponse, SecretPatch, SortDirection, SourcePolicy, TaskItemStatus, TaskItemsOptions,
    TaskItemsQuery, TaskListOptions, TaskListView, TaskSortBy,
};
use serde_json::{Value, json};
use uuid::Uuid;

#[derive(Debug, Clone)]
struct Captured {
    method: String,
    path: String,
    query: Option<String>,
    idempotency: Option<String>,
    auth: Option<String>,
}

type SharedLog = Arc<Mutex<Vec<Captured>>>;

fn record(method: &Method, uri: &OriginalUri, headers: &HeaderMap, log: &SharedLog) {
    let path = uri.path().to_string();
    let query = uri.query().map(str::to_string);
    let idempotency = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let auth = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    log.lock().expect("log").push(Captured {
        method: method.to_string(),
        path,
        query,
        idempotency,
        auth,
    });
}

fn task_id() -> String {
    "11111111-1111-1111-1111-111111111111".to_string()
}

fn file_id() -> String {
    "22222222-2222-2222-2222-222222222222".to_string()
}

fn now() -> String {
    "2026-01-01T00:00:00Z".to_string()
}

fn task_response_json(id: &str) -> Value {
    json!({
        "task_id": id,
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
        "created_at": now(),
        "started_at": now(),
        "finished_at": now(),
        "updated_at": now(),
        "deleted_at": null
    })
}

fn task_page_json(id: &str) -> Value {
    json!({
        "items": [task_response_json(id)],
        "pagination": {"page": 1, "page_size": 25, "total": 1, "total_pages": 1}
    })
}

fn task_items_json() -> Value {
    json!({
        "items": [{
            "item_id": task_id(),
            "ordinal": 0,
            "status": "succeeded",
            "resource_id": null,
            "file_id": null,
            "stage": null,
            "waiting_reason": null,
            "dependency_key": null,
            "next_attempt_at": null,
            "failure_stage": null,
            "error_message": null,
            "attempt_count": 1,
            "retryable": false,
            "created_at": now(),
            "started_at": now(),
            "finished_at": now()
        }],
        "next_cursor": null
    })
}

fn search_response_json() -> Value {
    json!({
        "query": "hello",
        "items": [{
            "chunk_id": task_id(),
            "document_id": 42,
            "group_key": "g",
            "group_path": "research",
            "visibility": "private",
            "source_key": "src",
            "external_id": "ext-1",
            "title": "Hello doc",
            "summary": "summary",
            "source_uri": "https://example.test/doc",
            "published_at": null,
            "chunk_index": 0,
            "chunk_text": "hello world chunk text for snippet",
            "score": 0.9,
            "metadata_json": {},
            "is_library_file": false,
            "is_fallback": false
        }],
        "pagination": {
            "page": 1,
            "page_size": 8,
            "total": 1,
            "total_pages": 1,
            "has_more": true,
            "total_is_exact": false,
            "next_cursor": "cursor-2",
            "prev_cursor": null
        }
    })
}

fn document_json() -> Value {
    json!({
        "document_id": 42,
        "group_key": "g",
        "group_path": "research",
        "visibility": "private",
        "source_key": "src",
        "external_id": "ext-1",
        "title": "Hello doc",
        "summary": null,
        "source_uri": "https://example.test/doc",
        "published_at": null,
        "updated_at": now(),
        "record_hash": "abc",
        "metadata_json": {},
        "is_library_file": false,
        "chunks": [{"chunk_id": task_id(), "chunk_index": 0, "text": "hello"}],
        "is_fallback": false
    })
}

fn ensure_scope_json() -> Value {
    json!({
        "group": {
            "group_id": 1,
            "group_key": "g",
            "group_path": "research",
            "name": "Research",
            "visibility": "private",
            "kind": "shared",
            "created_at": now(),
            "updated_at": now()
        },
        "metadata_indexes": []
    })
}

fn library_file_detail_json() -> Value {
    json!({
        "file_id": file_id(),
        "group_key": "g",
        "group_path": "research",
        "visibility": "private",
        "folder_path": "/",
        "filename": "a.pdf",
        "media_type": "application/pdf",
        "size_bytes": 10,
        "sha256": "abc",
        "source_available": true,
        "ingest_status": "succeeded",
        "created_at": now(),
        "updated_at": now(),
        "sections": []
    })
}

fn extraction_template_json() -> Value {
    json!({
        "template_key": "t",
        "version": 1,
        "description": null,
        "system_prompt": "p",
        "output_schema": {},
        "max_output_tokens": 100,
        "enabled": true,
        "created_at": now(),
        "updated_at": now()
    })
}

fn extraction_jobs_json() -> Value {
    json!({"jobs": [], "latest_results": []})
}

fn me_json() -> Value {
    json!({
        "user": {
            "user_id": 1,
            "login_name": "alice",
            "display_name": "Alice",
            "is_admin": false,
            "personal_group_path": "users/alice"
        }
    })
}

fn health_json() -> Value {
    json!({"status": "ok"})
}

async fn test_router(log: SharedLog) -> axum::Router {
    let log_clone = log.clone();

    axum::Router::new()
        .route(
            "/v1/scopes/ensure",
            post({
                let log = log.clone();
                move |method: Method, uri: OriginalUri, headers: HeaderMap, Json(_body): Json<Value>| {
                    let log = log.clone();
                    async move {
                        record(&method, &uri, &headers, &log);
                        (StatusCode::OK, Json(ensure_scope_json()))
                    }
                }
            }),
        )
        .route(
            "/v1/groups/by-path/{group_path}/batch/text",
            post({
                let log = log.clone();
                move |method: Method, uri: OriginalUri, headers: HeaderMap, Json(_body): Json<Value>| {
                    let log = log.clone();
                    async move {
                        record(&method, &uri, &headers, &log);
                        (StatusCode::ACCEPTED, Json(json!({"task_id": task_id(), "item_ids": []})))
                    }
                }
            }),
        )
        .route(
            "/v1/groups/by-path/{group_path}/batch/url",
            post({
                let log = log.clone();
                move |method: Method, uri: OriginalUri, headers: HeaderMap, Json(_body): Json<Value>| {
                    let log = log.clone();
                    async move {
                        record(&method, &uri, &headers, &log);
                        (StatusCode::ACCEPTED, Json(json!({"task_id": task_id(), "item_ids": []})))
                    }
                }
            }),
        )
        .route(
            "/v1/groups/by-path/{group_path}/batch/file",
            post({
                let log = log.clone();
                move |method: Method, uri: OriginalUri, headers: HeaderMap, Json(_body): Json<Value>| {
                    let log = log.clone();
                    async move {
                        record(&method, &uri, &headers, &log);
                        (StatusCode::ACCEPTED, Json(json!({"task_id": task_id(), "item_ids": []})))
                    }
                }
            }),
        )
        .route(
            "/v1/groups/by-path/{group_path}/batch/delete",
            post({
                let log = log.clone();
                move |method: Method, uri: OriginalUri, headers: HeaderMap, Json(_body): Json<Value>| {
                    let log = log.clone();
                    async move {
                        record(&method, &uri, &headers, &log);
                        (StatusCode::ACCEPTED, Json(json!({"task_id": task_id(), "item_ids": []})))
                    }
                }
            }),
        )
        .route(
            "/v1/groups/by-path/{group_path}/library/files/{file_id}/release-source",
            post({
                let log = log.clone();
                move |method: Method, uri: OriginalUri, headers: HeaderMap| {
                    let log = log.clone();
                    async move {
                        record(&method, &uri, &headers, &log);
                        (StatusCode::OK, Json(library_file_detail_json()))
                    }
                }
            }),
        )
        .route(
            "/v1/tasks",
            post({
                let log = log.clone();
                move |method: Method, uri: OriginalUri, headers: HeaderMap, Json(_body): Json<Value>| {
                    let log = log.clone();
                    async move {
                        record(&method, &uri, &headers, &log);
                        (StatusCode::ACCEPTED, Json(json!({"task_id": task_id(), "item_ids": []})))
                    }
                }
            })
            .get({
                let log = log.clone();
                move |method: Method, uri: OriginalUri, headers: HeaderMap| {
                    let log = log.clone();
                    async move {
                        record(&method, &uri, &headers, &log);
                        (StatusCode::OK, Json(task_page_json(&task_id())))
                    }
                }
            }),
        )
        .route(
            "/v1/tasks/{task_id}",
            get({
                let log = log.clone();
                move |method: Method, uri: OriginalUri, headers: HeaderMap| {
                    let log = log.clone();
                    async move {
                        let id = uri.path().split('/').nth(3).unwrap_or(&task_id()).to_string();
                        record(&method, &uri, &headers, &log);
                        (StatusCode::OK, Json(task_response_json(&id)))
                    }
                }
            })
            .delete({
                let log = log.clone();
                move |method: Method, uri: OriginalUri, headers: HeaderMap| {
                    let log = log.clone();
                    async move {
                        record(&method, &uri, &headers, &log);
                        StatusCode::NO_CONTENT
                    }
                }
            }),
        )
        .route(
            "/v1/tasks/{task_id}/items",
            get({
                let log = log.clone();
                move |method: Method, uri: OriginalUri, headers: HeaderMap| {
                    let log = log.clone();
                    async move {
                        record(&method, &uri, &headers, &log);
                        (StatusCode::OK, Json(task_items_json()))
                    }
                }
            }),
        )
        .route(
            "/v1/tasks/{task_id}/retry",
            post({
                let log = log.clone();
                move |method: Method, uri: OriginalUri, headers: HeaderMap| {
                    let log = log.clone();
                    async move {
                        record(&method, &uri, &headers, &log);
                        (
                            StatusCode::ACCEPTED,
                            Json(json!({"task": {"task_id": task_id(), "item_ids": []}, "retried_items": 1})),
                        )
                    }
                }
            }),
        )
        .route(
            "/v1/tasks/{task_id}/rerun",
            post({
                let log = log.clone();
                move |method: Method, uri: OriginalUri, headers: HeaderMap| {
                    let log = log.clone();
                    async move {
                        record(&method, &uri, &headers, &log);
                        (
                            StatusCode::ACCEPTED,
                            Json(json!({"task": {"task_id": task_id(), "item_ids": []}})),
                        )
                    }
                }
            }),
        )
        .route(
            "/v1/tasks/{task_id}/cancel",
            post({
                let log = log.clone();
                move |method: Method, uri: OriginalUri, headers: HeaderMap| {
                    let log = log.clone();
                    async move {
                        record(&method, &uri, &headers, &log);
                        StatusCode::NO_CONTENT
                    }
                }
            }),
        )
        .route(
            "/v1/tasks/{task_id}/trash",
            post({
                let log = log.clone();
                move |method: Method, uri: OriginalUri, headers: HeaderMap| {
                    let log = log.clone();
                    async move {
                        let id = uri.path().split('/').nth(3).unwrap_or(&task_id()).to_string();
                        record(&method, &uri, &headers, &log);
                        (StatusCode::OK, Json(task_response_json(&id)))
                    }
                }
            }),
        )
        .route(
            "/v1/tasks/{task_id}/restore",
            post({
                let log = log.clone();
                move |method: Method, uri: OriginalUri, headers: HeaderMap| {
                    let log = log.clone();
                    async move {
                        let id = uri.path().split('/').nth(3).unwrap_or(&task_id()).to_string();
                        record(&method, &uri, &headers, &log);
                        (StatusCode::OK, Json(task_response_json(&id)))
                    }
                }
            }),
        )
        .route(
            "/v1/admin/tasks/cancel-active",
            post({
                let log = log.clone();
                move |method: Method, uri: OriginalUri, headers: HeaderMap| {
                    let log = log.clone();
                    async move {
                        record(&method, &uri, &headers, &log);
                        (StatusCode::OK, Json(json!({"cancelled_tasks": 1})))
                    }
                }
            }),
        )
        .route(
            "/v1/search",
            post({
                let log = log.clone();
                move |method: Method, uri: OriginalUri, headers: HeaderMap, Json(_body): Json<Value>| {
                    let log = log.clone();
                    async move {
                        record(&method, &uri, &headers, &log);
                        (StatusCode::OK, Json(search_response_json()))
                    }
                }
            }),
        )
        .route(
            "/v1/documents/{document_id}",
            get({
                let log = log.clone();
                move |method: Method, uri: OriginalUri, headers: HeaderMap| {
                    let log = log.clone();
                    async move {
                        record(&method, &uri, &headers, &log);
                        (StatusCode::OK, Json(document_json()))
                    }
                }
            }),
        )
        .route(
            "/v1/groups/by-path/{group_path}/documents/by-external-id",
            get({
                let log = log.clone();
                move |method: Method, uri: OriginalUri, headers: HeaderMap| {
                    let log = log.clone();
                    async move {
                        record(&method, &uri, &headers, &log);
                        (StatusCode::OK, Json(document_json()))
                    }
                }
            }),
        )
        .route(
            "/v1/groups/by-path/{group_path}/documents/batch-get",
            post({
                let log = log.clone();
                move |method: Method, uri: OriginalUri, headers: HeaderMap, Json(_body): Json<Value>| {
                    let log = log.clone();
                    async move {
                        record(&method, &uri, &headers, &log);
                        (StatusCode::OK, Json(json!({"items": []})))
                    }
                }
            }),
        )
        .route(
            "/v1/groups/by-path/{group_path}/extraction-templates",
            get({
                let log = log.clone();
                move |method: Method, uri: OriginalUri, headers: HeaderMap| {
                    let log = log.clone();
                    async move {
                        record(&method, &uri, &headers, &log);
                        (StatusCode::OK, Json(json!([extraction_template_json()])))
                    }
                }
            })
            .put({
                let log = log.clone();
                move |method: Method, uri: OriginalUri, headers: HeaderMap, Json(_body): Json<Value>| {
                    let log = log.clone();
                    async move {
                        record(&method, &uri, &headers, &log);
                        (StatusCode::OK, Json(extraction_template_json()))
                    }
                }
            }),
        )
        .route(
            "/v1/groups/by-path/{group_path}/documents/{document_id}/extractions",
            get({
                let log = log.clone();
                move |method: Method, uri: OriginalUri, headers: HeaderMap| {
                    let log = log.clone();
                    async move {
                        record(&method, &uri, &headers, &log);
                        (StatusCode::OK, Json(extraction_jobs_json()))
                    }
                }
            }),
        )
        .route(
            "/v1/groups/by-path/{group_path}/documents/{document_id}/extractions/rebuild",
            post({
                let log = log.clone();
                move |method: Method, uri: OriginalUri, headers: HeaderMap, Json(_body): Json<Value>| {
                    let log = log.clone();
                    async move {
                        record(&method, &uri, &headers, &log);
                        (StatusCode::OK, Json(extraction_jobs_json()))
                    }
                }
            }),
        )
        .route(
            "/v1/auth/me",
            get({
                let log = log.clone();
                move |method: Method, uri: OriginalUri, headers: HeaderMap| {
                    let log = log.clone();
                    async move {
                        record(&method, &uri, &headers, &log);
                        (StatusCode::OK, Json(me_json()))
                    }
                }
            }),
        )
        .route(
            "/healthz",
            get({
                let log = log_clone;
                move |method: Method, uri: OriginalUri, headers: HeaderMap| {
                    let log = log.clone();
                    async move {
                        record(&method, &uri, &headers, &log);
                        (StatusCode::OK, Json(health_json()))
                    }
                }
            }),
        )
}

async fn test_client(log: &SharedLog) -> Context69Client {
    let router = test_router(log.clone()).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        axum::serve(listener, router).await.expect("serve");
    });
    Context69Client::builder()
        .base_url(&format!("http://{addr}"))
        .expect("base_url")
        .with_personal_access_token("ctx_pat_test-token-for-contract-paths")
        .expect("pat")
        .build()
        .expect("client")
}

fn last(log: &SharedLog) -> Captured {
    log.lock().expect("log").last().expect("request").clone()
}

fn query_map(query: &Option<String>) -> HashMap<String, String> {
    let mut out = HashMap::new();
    if let Some(q) = query {
        for pair in q.split('&') {
            if pair.is_empty() {
                continue;
            }
            let mut split = pair.splitn(2, '=');
            let key = split.next().unwrap_or("").to_string();
            let value = split.next().unwrap_or("").to_string();
            out.insert(key, value);
        }
    }
    out
}

#[test]
fn task_list_options_defaults_and_bounds_match_canonical_contracts() {
    let defaults = TaskListOptions::default();
    assert_eq!(defaults.view, TaskListView::Processing);
    assert_eq!(defaults.page, 1);
    assert_eq!(defaults.page_size, 25);
    assert!(defaults.query.is_none());
    assert!(defaults.as_canonical_query().is_ok());

    let canonical: CanonicalTaskListQuery = defaults.as_canonical_query().expect("valid");
    assert_eq!(canonical.page, 1);
    assert_eq!(canonical.page_size, 25);
    assert!(canonical.validate().is_ok());

    for (page, page_size) in [(0, 25), (1, 0), (10_001, 25), (1, 101)] {
        let bad = TaskListOptions {
            page,
            page_size,
            ..TaskListOptions::default()
        };
        assert!(
            bad.as_canonical_query().is_err(),
            "page={page} page_size={page_size} must be rejected"
        );
    }

    let with_filters = TaskListOptions {
        query: Some("hello".to_string()),
        sort_by: Some(TaskSortBy::CreatedAt),
        sort_direction: Some(SortDirection::Desc),
        ..TaskListOptions::default()
    };
    let canonical = with_filters.as_canonical_query().expect("filters");
    assert_eq!(canonical.query.as_deref(), Some("hello"));
    assert!(canonical.sort_by.is_some());
    assert!(canonical.sort_direction.is_some());
}

#[test]
fn task_items_options_default_has_no_hidden_limit_or_cursor() {
    let defaults = TaskItemsOptions::default();
    assert_eq!(defaults.limit, 100);
    assert!(defaults.cursor.is_none());
    assert!(defaults.status.is_none());
    assert!(defaults.as_wire_query().is_ok());

    let wire: TaskItemsQuery = defaults.as_wire_query().expect("valid");
    assert_eq!(wire.limit, 100);
    assert!(wire.cursor.is_none());
    assert!(wire.status.is_none());

    for limit in [0, 101, 200] {
        let bad = TaskItemsOptions {
            limit,
            cursor: None,
            status: None,
        };
        assert!(
            bad.as_wire_query().is_err(),
            "limit={limit} must be rejected (canonical 1..=100, no hidden 200)"
        );
    }

    let with_cursor = TaskItemsOptions {
        limit: 25,
        cursor: Some("25".to_string()),
        status: None,
    };
    let wire = with_cursor.as_wire_query().expect("cursor");
    assert_eq!(wire.limit, 25);
    assert_eq!(wire.cursor.as_deref(), Some("25"));
    assert!(wire.status.is_none());

    // Issue 413 Phase 1: SDK parity for the `status` filter; cursor stays
    // scoped to the filter (reset on filter change, client-side contract).
    let filtered = TaskItemsOptions {
        limit: 25,
        cursor: Some("25".to_string()),
        status: Some(TaskItemStatus::Failed),
    };
    let wire = filtered.as_wire_query().expect("status filter");
    assert_eq!(wire.status, Some(TaskItemStatus::Failed));
    assert_eq!(wire.cursor.as_deref(), Some("25"));
}

#[test]
fn sdk_only_construction_covers_facade_inputs() {
    // Every type needed to construct a facade call comes from `context69_sdk`
    // alone; no direct `context69_contracts` import appears in this file.
    let _view = TaskListView::Completed;
    let _direction = SortDirection::Asc;
    let _items_query = TaskItemsQuery {
        limit: 10,
        cursor: None,
        status: None,
    };
    let _policy = SourcePolicy::Retain;
    let _ingest = IngestOptions::retain();
    let _secret = SecretPatch::Keep;
    let _canonical_search = CanonicalSearchRequest {
        query: "hello".to_string(),
        locale: None,
        limit: 8,
        source_key: None,
        group_path: None,
        published_after: None,
        published_before: None,
        cursor: None,
        metadata_filters: Vec::new(),
        sort: context69_sdk::SearchSort::Relevance,
    };
    assert!(_canonical_search.validate().is_ok());
    let _canonical_upload = CanonicalUploadMetadata::default();
    let _canonical_settings: CanonicalUpdateSearchSettingsRequest = serde_json::from_value(json!({
        "mode": "hybrid",
        "rerank_enabled": false,
        "rerank_base_url": "",
        "rerank_model": "",
        "candidate_limit": 10,
        "timeout_secs": 30,
        "api_key": {"op": "keep"}
    }))
    .expect("canonical settings decode SDK-only");
    let _pagination = SearchPagination {
        page: 1,
        page_size: 8,
        total: 0,
        total_pages: 0,
        has_more: None,
        total_is_exact: None,
        next_cursor: None,
        prev_cursor: None,
    };
    let _key = DocumentKey {
        source_key: "src".to_string(),
        external_id: "ext".to_string(),
    };
}

#[tokio::test]
async fn compact_search_preserves_pagination_metadata() {
    let log: SharedLog = Arc::new(Mutex::new(Vec::new()));
    let client = test_client(&log).await;
    let request: SearchRequest =
        serde_json::from_value(json!({"query": "hello"})).expect("search request");
    let compact = client.search_compact(&request).await.expect("compact");
    assert_eq!(compact.query, "hello");
    assert_eq!(compact.hits.len(), 1);
    assert_eq!(compact.pagination.next_cursor.as_deref(), Some("cursor-2"));
    assert_eq!(compact.pagination.has_more, Some(true));
    assert_eq!(compact.next_cursor(), Some("cursor-2"));
    assert!(compact.has_more());

    let full: SearchResponse = client.search(&request).await.expect("search");
    assert_eq!(full.pagination.next_cursor.as_deref(), Some("cursor-2"));
    assert_eq!(full.pagination.has_more, Some(true));
}

#[tokio::test]
async fn facade_emits_canonical_paths_methods_and_idempotency() {
    let log: SharedLog = Arc::new(Mutex::new(Vec::new()));
    let client = test_client(&log).await;
    let group = "research";
    let task_uuid = Uuid::parse_str(&task_id()).expect("task uuid");
    let file_uuid = Uuid::parse_str(&file_id()).expect("file uuid");

    let scope: context69_sdk::ScopeSpec = serde_json::from_value(
        json!({"group_path": group, "name": "Research", "visibility": "private"}),
    )
    .expect("scope");
    client.ensure_scope(&scope).await.expect("ensure_scope");
    assert_eq!(last(&log).method, "POST");
    assert_eq!(last(&log).path, "/v1/scopes/ensure");
    assert!(last(&log).idempotency.is_none());

    let text: context69_sdk::TextBatchRequest = serde_json::from_value(json!({
        "items": [{"external_id": "a", "title": "t", "content": "c"}]
    }))
    .expect("text batch");
    client
        .submit_text_batch(group, &text)
        .await
        .expect("submit_text_batch");
    assert_eq!(last(&log).method, "POST");
    assert_eq!(
        last(&log).path,
        format!("/v1/groups/by-path/{group}/batch/text")
    );
    let text_key = last(&log).idempotency.clone().expect("idempotency");
    assert!(text_key.starts_with("ctx69-sdk-"));
    client
        .submit_text_batch(group, &text)
        .await
        .expect("text_batch repeat");
    assert_eq!(last(&log).idempotency.as_deref(), Some(text_key.as_str()));

    let url_batch: context69_sdk::UrlBatchRequest = serde_json::from_value(json!({
        "items": [{"url": "https://example.test/a", "options": {"metadata": {}, "source_policy": "retain"}}]
    }))
    .expect("url batch");
    client
        .submit_url_batch(group, &url_batch)
        .await
        .expect("url_batch");
    assert_eq!(last(&log).method, "POST");
    assert!(last(&log).path.ends_with("/batch/url"));
    assert!(last(&log).idempotency.is_some());

    let file_batch: context69_sdk::FileBatchRequest = serde_json::from_value(json!({
        "items": [{"filename": "a.txt", "media_type": "text/plain", "content_base64": "aGk=", "options": {"metadata": {}, "source_policy": "retain"}}]
    }))
    .expect("file batch");
    client
        .submit_file_batch(group, &file_batch)
        .await
        .expect("file_batch");
    assert_eq!(last(&log).method, "POST");
    assert!(last(&log).path.ends_with("/batch/file"));
    assert!(last(&log).idempotency.is_some());

    let delete_batch: context69_sdk::DeleteBatchRequest = serde_json::from_value(json!({
        "items": [{"source_key": "src", "external_id": "ext"}]
    }))
    .expect("delete batch");
    client
        .submit_delete_batch(group, &delete_batch)
        .await
        .expect("delete_batch");
    assert_eq!(last(&log).method, "POST");
    assert!(last(&log).path.ends_with("/batch/delete"));
    assert!(last(&log).idempotency.is_some());

    // Distinct bodies must not share an idempotency key.
    let other_text: context69_sdk::TextBatchRequest = serde_json::from_value(json!({
        "items": [{"external_id": "b", "title": "t", "content": "c"}]
    }))
    .expect("other text");
    client
        .submit_text_batch(group, &other_text)
        .await
        .expect("other");
    assert_ne!(last(&log).idempotency.as_deref(), Some(text_key.as_str()));

    client
        .release_file_source(group, file_uuid)
        .await
        .expect("release");
    assert_eq!(last(&log).method, "POST");
    assert_eq!(
        last(&log).path,
        format!("/v1/groups/by-path/{group}/library/files/{file_uuid}/release-source")
    );
    assert!(last(&log).idempotency.is_none());

    let submit: context69_sdk::TaskSubmitRequest =
        serde_json::from_value(json!({"kind": "vector_rebuild"})).expect("submit");
    client.submit_task(&submit).await.expect("submit_task");
    assert_eq!(last(&log).method, "POST");
    assert_eq!(last(&log).path, "/v1/tasks");
    assert!(last(&log).idempotency.is_some());

    client.get_task(task_uuid).await.expect("get_task");
    assert_eq!(last(&log).method, "GET");
    assert_eq!(last(&log).path, format!("/v1/tasks/{task_uuid}"));
    assert!(last(&log).idempotency.is_none());

    client
        .list_tasks(&TaskListOptions::default())
        .await
        .expect("list_tasks");
    assert_eq!(last(&log).method, "GET");
    assert_eq!(last(&log).path, "/v1/tasks");
    let query = query_map(&last(&log).query);
    assert_eq!(query.get("view").map(String::as_str), Some("processing"));
    assert_eq!(query.get("page").map(String::as_str), Some("1"));
    assert_eq!(query.get("page_size").map(String::as_str), Some("25"));
    assert!(last(&log).idempotency.is_none());

    client
        .list_task_items(task_uuid, &TaskItemsOptions::default())
        .await
        .expect("list_task_items default");
    assert_eq!(last(&log).method, "GET");
    assert_eq!(last(&log).path, format!("/v1/tasks/{task_uuid}/items"));
    let query = query_map(&last(&log).query);
    assert_eq!(query.get("limit").map(String::as_str), Some("100"));
    assert!(
        !query.contains_key("cursor"),
        "default must omit cursor, not send 0"
    );
    assert!(
        !query.contains_key("status"),
        "default must omit status (lists every status)"
    );
    assert!(!last(&log).query.as_deref().unwrap_or("").contains("200"));

    client
        .list_task_items(
            task_uuid,
            &TaskItemsOptions {
                limit: 25,
                cursor: Some("25".to_string()),
                status: None,
            },
        )
        .await
        .expect("list_task_items cursor");
    let query = query_map(&last(&log).query);
    assert_eq!(query.get("limit").map(String::as_str), Some("25"));
    assert_eq!(query.get("cursor").map(String::as_str), Some("25"));
    assert!(
        !query.contains_key("status"),
        "unset filter must omit status"
    );

    // Issue 413 Phase 1: filtered window sends `status`; cursor stays scoped
    // to the filter (client resets cursor when the filter changes).
    client
        .list_task_items(
            task_uuid,
            &TaskItemsOptions {
                limit: 25,
                cursor: Some("25".to_string()),
                status: Some(TaskItemStatus::Failed),
            },
        )
        .await
        .expect("list_task_items status");
    let query = query_map(&last(&log).query);
    assert_eq!(query.get("limit").map(String::as_str), Some("25"));
    assert_eq!(query.get("cursor").map(String::as_str), Some("25"));
    assert_eq!(query.get("status").map(String::as_str), Some("failed"));

    client.retry_task(task_uuid).await.expect("retry");
    assert_eq!(last(&log).method, "POST");
    assert_eq!(last(&log).path, format!("/v1/tasks/{task_uuid}/retry"));

    client.rerun_task(task_uuid).await.expect("rerun");
    assert_eq!(last(&log).method, "POST");
    assert_eq!(last(&log).path, format!("/v1/tasks/{task_uuid}/rerun"));

    client.cancel_task(task_uuid).await.expect("cancel");
    assert_eq!(last(&log).method, "POST");
    assert_eq!(last(&log).path, format!("/v1/tasks/{task_uuid}/cancel"));

    client.trash_task(task_uuid).await.expect("trash");
    assert_eq!(last(&log).method, "POST");
    assert_eq!(last(&log).path, format!("/v1/tasks/{task_uuid}/trash"));

    client.restore_task(task_uuid).await.expect("restore");
    assert_eq!(last(&log).method, "POST");
    assert_eq!(last(&log).path, format!("/v1/tasks/{task_uuid}/restore"));

    client
        .purge_trashed_task(task_uuid)
        .await
        .expect("purge_trashed_task");
    assert_eq!(last(&log).method, "DELETE");
    assert_eq!(last(&log).path, format!("/v1/tasks/{task_uuid}"));

    client
        .cancel_active_tasks()
        .await
        .expect("cancel_active_tasks");
    assert_eq!(last(&log).method, "POST");
    assert_eq!(last(&log).path, "/v1/admin/tasks/cancel-active");

    let search_req: SearchRequest =
        serde_json::from_value(json!({"query": "hello"})).expect("search");
    client.search(&search_req).await.expect("search");
    assert_eq!(last(&log).method, "POST");
    assert_eq!(last(&log).path, "/v1/search");
    assert!(last(&log).idempotency.is_none());

    client.get_document(42, None).await.expect("get_document");
    assert_eq!(last(&log).method, "GET");
    assert_eq!(last(&log).path, "/v1/documents/42");

    let key = DocumentKey {
        source_key: "src".to_string(),
        external_id: "ext".to_string(),
    };
    client
        .get_document_by_key(group, &key, None)
        .await
        .expect("by key");
    assert_eq!(last(&log).method, "GET");
    assert_eq!(
        last(&log).path,
        format!("/v1/groups/by-path/{group}/documents/by-external-id")
    );

    let batch_get = BatchGetDocumentsRequest {
        keys: vec![key],
        locale: None,
    };
    client
        .get_documents(group, &batch_get)
        .await
        .expect("batch get");
    assert_eq!(last(&log).method, "POST");
    assert_eq!(
        last(&log).path,
        format!("/v1/groups/by-path/{group}/documents/batch-get")
    );

    client
        .list_extraction_templates(group)
        .await
        .expect("list templates");
    assert_eq!(last(&log).method, "GET");
    assert_eq!(
        last(&log).path,
        format!("/v1/groups/by-path/{group}/extraction-templates")
    );

    let template_in: context69_sdk::ExtractionTemplateInput = serde_json::from_value(json!({
        "template_key": "t",
        "system_prompt": "p",
        "output_schema": {"type": "object"}
    }))
    .expect("template input");
    client
        .upsert_extraction_template(group, &template_in)
        .await
        .expect("upsert template");
    assert_eq!(last(&log).method, "PUT");

    client
        .document_extractions(group, 42)
        .await
        .expect("document extractions");
    assert_eq!(last(&log).method, "GET");
    assert_eq!(
        last(&log).path,
        format!("/v1/groups/by-path/{group}/documents/42/extractions")
    );

    let rebuild = RebuildDocumentExtractionsRequest {
        template_keys: Vec::new(),
    };
    client
        .rebuild_document_extractions(group, 42, &rebuild)
        .await
        .expect("rebuild");
    assert_eq!(last(&log).method, "POST");
    assert!(last(&log).path.ends_with("/extractions/rebuild"));

    let me: AuthMeResponse = client.me().await.expect("me");
    assert_eq!(me.user.login_name, "alice");
    assert_eq!(last(&log).method, "GET");
    assert_eq!(last(&log).path, "/v1/auth/me");

    client.healthz().await.expect("healthz");
    assert_eq!(last(&log).method, "GET");
    assert_eq!(last(&log).path, "/healthz");
    assert!(
        last(&log).auth.is_none(),
        "healthz must stay unauthenticated"
    );
}

#[test]
fn task_list_options_rejects_out_of_bounds_before_any_io() {
    for (page, page_size) in [(0, 25), (10_001, 25), (1, 0), (1, 101)] {
        let options = TaskListOptions {
            page,
            page_size,
            ..TaskListOptions::default()
        };
        assert!(options.validate().is_err());
        assert!(options.as_canonical_query().is_err());
    }
}
