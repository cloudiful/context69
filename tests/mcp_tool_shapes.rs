//! MCP tool-shape tests (Redmine 362 Task 5 / mcp-boundary).
//!
//! These tests pin the six frozen tool input/output shapes, the progressive
//! search -> detail flow, anonymous/public source behavior, and invalid
//! cursor/limit handling — all without a database.

use chrono::Utc;
use context69::contracts::{
    DocumentChunkResponse, DocumentKey, DocumentResponse, MCP_TOOL_NAMES, McpBatchDocumentArgs,
    McpBatchDocumentKeys, McpDocumentArgs, McpDocumentKeyArgs, McpDocumentQueryArgs,
    McpDocumentQueryResponse, McpSearchRequest, McpSearchResponse, McpSourceListArgs,
    McpSourceSummary, SourceStatus, Visibility, paginate_document_detail,
    paginate_source_summaries,
};
use serde_json::{Value, json};
use uuid::Uuid;

fn schema_of<T: schemars::JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T)).expect("schema serializes")
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

fn public_status(source_key: &str) -> SourceStatus {
    SourceStatus {
        group_key: "g".to_string(),
        group_path: "research/news".to_string(),
        visibility: Visibility::Public,
        source_key: source_key.to_string(),
        display_name: format!("{source_key} display"),
        description: None,
        example_queries: Vec::new(),
        connection: "primary-ro".to_string(),
        has_database_url: true,
        origin_status: context69::contracts::SourceOriginStatusKind::Connected,
        origin_message: Some("must not leak".to_string()),
        sync_strategy: context69::contracts::SourceSyncStrategy::Cursor,
        connector_type: context69::contracts::SourceConnectorType::PostgresSql,
        base_query: "SELECT secret FROM t".to_string(),
        batch_size: 100,
        last_cursor_updated_at: None,
        last_cursor_external_id: None,
        last_success_at: None,
    }
}

fn private_status(source_key: &str) -> SourceStatus {
    SourceStatus {
        visibility: Visibility::Private,
        ..public_status(source_key)
    }
}

/// Mirror of the adapter's anonymous gate: anonymous callers keep public
/// sources only; authenticated callers keep everything.
fn visible_for(statuses: &[SourceStatus], user_id: Option<i64>) -> Vec<SourceStatus> {
    let mut visible = statuses.to_vec();
    if user_id.is_none() {
        visible.retain(|source| source.visibility == Visibility::Public);
    }
    visible
}

fn sample_document() -> DocumentResponse {
    DocumentResponse {
        document_id: 421,
        group_key: "g".to_string(),
        group_path: "research/news".to_string(),
        visibility: Visibility::Public,
        source_key: "news-pg".to_string(),
        external_id: "art-421".to_string(),
        title: "Rates decision".to_string(),
        summary: Some("Summary".to_string()),
        source_uri: "postgres://articles/421".to_string(),
        published_at: Some(Utc::now()),
        updated_at: Utc::now(),
        record_hash: "hash".to_string(),
        metadata_json: json!({}),
        library_file_id: None,
        library_section_label: None,
        library_path: None,
        is_library_file: false,
        chunks: vec![DocumentChunkResponse {
            chunk_id: Uuid::new_v4(),
            chunk_index: 0,
            text: "chunk body".to_string(),
        }],
        requested_locale: None,
        content_locale: None,
        translation_status: None,
        is_fallback: false,
    }
}

#[test]
fn frozen_tool_names_are_stable() {
    assert_eq!(
        MCP_TOOL_NAMES,
        [
            "search_documents",
            "get_document",
            "query_documents",
            "get_document_by_external_id",
            "get_documents",
            "list_sources",
        ]
    );
    let mut sorted = MCP_TOOL_NAMES.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), MCP_TOOL_NAMES.len(), "tool names are unique");
}

#[test]
fn search_tool_accepts_documented_payloads() {
    // Example from docs/mcp.md must execute against the tool contract.
    let documented: McpSearchRequest = serde_json::from_value(json!({
        "query": "央行降准对银行股的影响",
        "limit": 8,
        "group_path": "research/news",
    }))
    .expect("documented search example deserializes");
    assert!(documented.validate().is_ok());
    let internal = documented.to_search_request();
    assert_eq!(internal.limit, 8);

    let full: McpSearchRequest = serde_json::from_value(json!({
        "query": "rates",
        "limit": 8,
        "group_path": "research/news",
        "source_key": "news-pg",
        "locale": "zh-CN",
        "cursor": null,
    }))
    .expect("full search payload deserializes");
    assert!(full.validate().is_ok());

    let existed_before_v016 = json!({
        "query": "rates",
        "limit": 8,
        "page": 2,
    });
    assert!(
        serde_json::from_value::<McpSearchRequest>(existed_before_v016).is_err(),
        "deprecated page must be rejected, not silently reinterpreted"
    );

    let response_schema = schema_of::<McpSearchResponse>();
    assert_eq!(
        prop_names(&response_schema),
        vec!["has_more", "hits", "next_cursor"]
    );
}

#[test]
fn search_to_detail_progressive_flow() {
    // A search hit carries the identifiers a detail call needs — nothing more.
    let hit = context69::contracts::McpSearchHit {
        document_id: 421,
        external_id: "art-421".to_string(),
        title: "Rates decision".to_string(),
        summary: None,
        source_uri: "postgres://articles/421".to_string(),
        published_at: None,
        score: 0.9,
        snippet: "snippet".to_string(),
    };
    let detail_args = McpDocumentArgs {
        document_id: hit.document_id,
        locale: None,
        chunk_cursor: None,
        chunk_limit: 20,
    };
    assert!(detail_args.validate().is_ok());

    let document = sample_document();
    assert_eq!(document.document_id, hit.document_id);
    assert_eq!(document.external_id, hit.external_id);
    let detail = paginate_document_detail(
        &document,
        detail_args.start_offset().unwrap(),
        detail_args.chunk_limit,
    )
    .expect("detail window");
    assert_eq!(detail.document.document_id, hit.document_id);
    assert!(!detail.has_more);

    // Documented chunk-continuation payload executes against the contract.
    let continued: McpDocumentArgs = serde_json::from_value(json!({
        "document_id": 421,
        "chunk_cursor": "20",
        "chunk_limit": 20,
    }))
    .expect("documented continuation example deserializes");
    assert!(continued.validate().is_ok());
}

#[test]
fn anonymous_callers_only_see_public_sources() {
    let statuses = vec![
        public_status("news-pg"),
        private_status("internal-pg"),
        public_status("docs-fs"),
    ];
    let anonymous = visible_for(&statuses, None);
    assert_eq!(anonymous.len(), 2);
    assert!(
        anonymous
            .iter()
            .all(|source| source.visibility == Visibility::Public)
    );

    let summaries: Vec<McpSourceSummary> = anonymous
        .iter()
        .map(McpSourceSummary::from_status)
        .collect();
    let serialized = serde_json::to_string(&summaries).expect("summaries serialize");
    for leaked in [
        "primary-ro",
        "must not leak",
        "SELECT secret",
        "has_database_url",
    ] {
        assert!(
            !serialized.contains(leaked),
            "operational value leaked into anonymous listing: {leaked}"
        );
    }

    let authenticated = visible_for(&statuses, Some(7));
    assert_eq!(authenticated.len(), 3);

    // Safe summaries still paginate with continuation for anonymous callers.
    let page = paginate_source_summaries(
        &summaries,
        &McpSourceListArgs {
            limit: 1,
            cursor: None,
        },
    )
    .expect("first anonymous page");
    assert_eq!(page.sources.len(), 1);
    assert!(page.has_more);
    assert_eq!(page.next_cursor.as_deref(), Some("1"));
}

#[test]
fn query_tool_shape_and_cursor_flow() {
    let args: McpDocumentQueryArgs = serde_json::from_value(json!({
        "group_path": "research/news",
        "query": {
            "source_key": "news-pg",
            "metadata_filters": [],
            "sort": [],
            "limit": 20,
            "cursor": null,
        },
    }))
    .expect("query tool payload deserializes");
    assert!(args.validate().is_ok());

    let response = McpDocumentQueryResponse::new(Vec::new(), Some("opaque".to_string()));
    assert!(response.has_more);
    response
        .validate_continuation()
        .expect("valid continuation");
    let terminal = McpDocumentQueryResponse::new(Vec::new(), None);
    assert!(!terminal.has_more);
}

#[test]
fn key_and_batch_tool_shapes() {
    let key_args: McpDocumentKeyArgs = serde_json::from_value(json!({
        "group_path": "research/news",
        "key": {"source_key": "news-pg", "external_id": "art-1"},
    }))
    .expect("key tool payload deserializes");
    assert!(key_args.validate().is_ok());

    let batch_args: McpBatchDocumentArgs = serde_json::from_value(json!({
        "group_path": "research/news",
        "request": {
            "keys": [
                {"source_key": "news-pg", "external_id": "art-1"},
                {"source_key": "news-pg", "external_id": "art-2"},
            ],
        },
    }))
    .expect("batch tool payload deserializes");
    assert!(batch_args.validate().is_ok());
}

#[test]
fn list_sources_tool_shape_with_pagination() {
    // Documented examples execute against the tool contract.
    let first: McpSourceListArgs = serde_json::from_value(json!({"limit": 50}))
        .expect("documented list_sources example deserializes");
    assert!(first.validate().is_ok());
    let continued: McpSourceListArgs = serde_json::from_value(json!({"limit": 50, "cursor": "50"}))
        .expect("documented continuation example deserializes");
    assert!(continued.validate().is_ok());
    assert_eq!(continued.start_offset().expect("offset"), 50);

    // Bare `{}` keeps working through defaults.
    let bare: McpSourceListArgs =
        serde_json::from_value(json!({})).expect("empty params use defaults");
    assert_eq!(bare.limit, 50);
    assert!(bare.cursor.is_none());
}

#[test]
fn invalid_cursor_and_limit_are_rejected() {
    let bad_search = McpSearchRequest {
        query: "rates".to_string(),
        group_path: None,
        source_key: None,
        locale: None,
        limit: 99,
        cursor: None,
    };
    assert!(bad_search.validate().is_err());

    // Search cursors are opaque service tokens: well-formed strings pass through
    // to the search service (which rejects cross-ordering cursors), while blank
    // or oversized values fail at the MCP boundary.
    let opaque_passthrough = McpSearchRequest {
        query: "rates".to_string(),
        group_path: None,
        source_key: None,
        locale: None,
        limit: 8,
        cursor: Some("not-a-search-cursor-!!!".to_string()),
    };
    assert!(opaque_passthrough.validate().is_ok());
    let blank_cursor = McpSearchRequest {
        cursor: Some("".to_string()),
        ..opaque_passthrough.clone()
    };
    assert!(blank_cursor.validate().is_err());

    let bad_source_cursor = McpSourceListArgs {
        limit: 10,
        cursor: Some("10.5".to_string()),
    };
    assert!(bad_source_cursor.validate().is_err());

    let bad_chunk = McpDocumentArgs {
        document_id: 1,
        locale: None,
        chunk_cursor: Some("-1".to_string()),
        chunk_limit: 20,
    };
    assert!(bad_chunk.validate().is_err());

    let bad_query = McpDocumentQueryArgs {
        group_path: "research/news".to_string(),
        query: context69::contracts::McpDocumentQuery {
            locale: None,
            source_key: None,
            published_after: None,
            published_before: None,
            metadata_filters: Vec::new(),
            sort: Vec::new(),
            limit: 0,
            cursor: None,
        },
    };
    assert!(bad_query.validate().is_err());

    let bad_batch = McpBatchDocumentArgs {
        group_path: "research/news".to_string(),
        request: McpBatchDocumentKeys {
            keys: Vec::new(),
            locale: None,
        },
    };
    assert!(bad_batch.validate().is_err());

    // Oversized key lists are rejected at deserialization, before validation.
    let too_many_keys = serde_json::to_value(
        &(0..21)
            .map(|index| DocumentKey {
                source_key: "news-pg".to_string(),
                external_id: format!("art-{index}"),
            })
            .collect::<Vec<_>>(),
    )
    .unwrap();
    assert!(
        serde_json::from_value::<McpBatchDocumentKeys>(json!({"keys": too_many_keys})).is_err()
    );
}

#[test]
fn query_tool_omitted_limit_docs_example() {
    // Documented query_documents example (no nested limit) executes against
    // the tool contract with the MCP-local default of 20.
    let args: McpDocumentQueryArgs = serde_json::from_value(json!({
        "group_path": "research/news",
        "query": {"source_key": "news-pg"},
    }))
    .expect("documented query example deserializes");
    assert_eq!(args.query.limit, 20);
    assert!(args.validate().is_ok());
}

#[test]
fn batch_tool_docs_example() {
    // Documented get_documents example executes against the tool contract.
    let args: McpBatchDocumentArgs = serde_json::from_value(json!({
        "group_path": "research/news",
        "request": {
            "keys": [
                {"source_key": "news-pg", "external_id": "art-1"},
                {"source_key": "news-pg", "external_id": "art-2"},
            ],
        },
    }))
    .expect("documented batch example deserializes");
    assert!(args.validate().is_ok());
}

#[test]
fn document_key_type_is_shared_with_batch() {
    // The same key shape flows from search hits through batch lookups.
    let key = DocumentKey {
        source_key: "news-pg".to_string(),
        external_id: "art-421".to_string(),
    };
    let roundtrip: DocumentKey =
        serde_json::from_value(serde_json::to_value(&key).unwrap()).unwrap();
    assert_eq!(roundtrip.external_id, "art-421");
}
