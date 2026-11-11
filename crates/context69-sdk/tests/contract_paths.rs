//! SDK-only construction and validation units for canonical task and facade
//! inputs.
//!
//! Every type used here comes from `context69_sdk`; no server or transport is
//! involved.

use context69_sdk::{
    CanonicalSearchRequest, CanonicalTaskListQuery, CanonicalUpdateSearchSettingsRequest,
    CanonicalUploadMetadata, CreateDocumentResultRequest, DocumentKey, DocumentResultProvenance,
    DocumentResultResponse, IngestOptions, ListDocumentResultsQuery, ListDocumentResultsResponse,
    SearchPagination, SecretPatch, SortDirection, SourcePolicy, TaskItemStatus, TaskItemsOptions,
    TaskItemsQuery, TaskListOptions, TaskListView, TaskSortBy,
};
use serde_json::json;

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
    // Issue #910 P1: the whole result envelope is constructible from the SDK.
    let _result_request = CreateDocumentResultRequest {
        producer: "stock.news".to_string(),
        result_kind: "sentiment.v1".to_string(),
        schema_ref: "stock.news/sentiment@1".to_string(),
        source_record_hash: "sha256:abc".to_string(),
        idempotency_key: "run-1".to_string(),
        result: json!({"direction": "positive"}),
        provenance: Some(DocumentResultProvenance {
            model_name: Some("m".to_string()),
            workflow_version: None,
            run_id: Some("run-1".to_string()),
            producer_config_hash: Some("cfg".to_string()),
        }),
    };
    let _result_query = ListDocumentResultsQuery {
        producer: Some("stock.news".to_string()),
        result_kind: None,
        after_id: Some(9),
        current_only: true,
        latest_only: true,
        limit: 50,
    };
    let _result_page: ListDocumentResultsResponse = serde_json::from_value(json!({
        "source_record_hash": "sha256:abc",
        "items": [{
            "id": 9, "document_id": 42, "author_user_id": 1, "producer": "stock.news",
            "result_kind": "sentiment.v1", "schema_ref": "s",
            "source_record_hash": "sha256:abc", "idempotency_key": "run-1",
            "result": {}, "created_at": "2026-01-01T00:00:00Z"
        }],
        "next_after_id": null
    }))
    .expect("result page decodes SDK-only");
    let _result: DocumentResultResponse = _result_page.items[0].clone();
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
