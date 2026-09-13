//! MCP contract tests (Redmine 362 Task 5 / mcp-boundary).
//!
//! Pure contract checks with no database: JSON Schemas expose only intended
//! fields with truthful bounds, runtime caps match the schemas, continuation
//! invariants hold, and operational source fields never cross the boundary.

use chrono::Utc;
use context69::contracts::{
    DocumentChunkResponse, DocumentKey, DocumentResponse, McpBatchDocumentArgs,
    McpBatchDocumentKeys, McpDocumentArgs, McpDocumentDetailResponse, McpDocumentKeyArgs,
    McpDocumentQueryArgs, McpSearchRequest, McpSearchResponse, McpSourceListArgs,
    McpSourceListResponse, McpSourceSummary, SearchHit, SourceStatus, Visibility,
    paginate_document_detail, paginate_source_summaries, parse_chunk_cursor, parse_offset_cursor,
};
use serde_json::{Value, json};
use uuid::Uuid;

fn schema_of<T: schemars::JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T)).expect("schema serializes")
}

fn collect_keyword(node: &Value, keyword: &str, out: &mut Vec<Value>) {
    match node {
        Value::Object(map) => {
            for (key, value) in map {
                if key == keyword {
                    out.push(value.clone());
                } else {
                    collect_keyword(value, keyword, out);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_keyword(item, keyword, out);
            }
        }
        _ => {}
    }
}

/// Every occurrence of `keyword` inside the subschema for `prop`.
fn prop_keywords(schema: &Value, prop: &str, keyword: &str) -> Vec<Value> {
    let mut out = Vec::new();
    if let Some(prop_schema) = schema.get("properties").and_then(|p| p.get(prop)) {
        collect_keyword(prop_schema, keyword, &mut out);
    }
    out
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

fn sample_status(visibility: Visibility) -> SourceStatus {
    SourceStatus {
        group_key: "g".to_string(),
        group_path: "research/news".to_string(),
        visibility,
        source_key: "news-pg".to_string(),
        display_name: "News Postgres".to_string(),
        description: Some("Upstream news warehouse".to_string()),
        example_queries: vec!["latest rates".to_string()],
        connection: "primary-ro".to_string(),
        has_database_url: true,
        origin_status: context69::contracts::SourceOriginStatusKind::Connected,
        origin_message: Some("replica lag 2s".to_string()),
        sync_strategy: context69::contracts::SourceSyncStrategy::Cursor,
        connector_type: context69::contracts::SourceConnectorType::PostgresSql,
        base_query: "SELECT id, title FROM articles".to_string(),
        batch_size: 500,
        last_cursor_updated_at: Some(Utc::now()),
        last_cursor_external_id: Some("art-999".to_string()),
        last_success_at: Some(Utc::now()),
    }
}

fn sample_document(chunks: usize, chunk_len: usize) -> DocumentResponse {
    DocumentResponse {
        document_id: 421,
        group_key: "g".to_string(),
        group_path: "research/news".to_string(),
        visibility: Visibility::Public,
        source_key: "news-pg".to_string(),
        external_id: "art-421".to_string(),
        title: "Rates decision".to_string(),
        summary: Some("Central bank decision summary".to_string()),
        source_uri: "postgres://articles/421".to_string(),
        published_at: Some(Utc::now()),
        updated_at: Utc::now(),
        record_hash: "deadbeef".to_string(),
        metadata_json: json!({"desk": "markets"}),
        library_file_id: None,
        library_section_label: None,
        library_path: None,
        is_library_file: false,
        chunks: (0..chunks)
            .map(|index| DocumentChunkResponse {
                chunk_id: Uuid::new_v4(),
                chunk_index: index as i32,
                text: "x".repeat(chunk_len),
            })
            .collect(),
        requested_locale: None,
        content_locale: None,
        translation_status: None,
        is_fallback: false,
    }
}

fn sample_query_request(limit: u8) -> context69::contracts::McpDocumentQuery {
    context69::contracts::McpDocumentQuery {
        locale: None,
        source_key: Some("news-pg".to_string()),
        published_after: None,
        published_before: None,
        metadata_filters: Vec::new(),
        sort: Vec::new(),
        limit,
        cursor: None,
    }
}

/// Collect every `$defs`/`definitions` entry name and `$ref` target in a
/// schema document.
fn collect_schema_refs(node: &Value, out: &mut Vec<String>) {
    match node {
        Value::Object(map) => {
            for key in ["$defs", "definitions"] {
                if let Some(defs) = map.get(key).and_then(Value::as_object) {
                    out.extend(defs.keys().cloned());
                }
            }
            for (key, value) in map {
                if key == "$ref"
                    && let Some(target) = value.as_str()
                {
                    out.push(target.to_string());
                } else {
                    collect_schema_refs(value, out);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_schema_refs(item, out);
            }
        }
        _ => {}
    }
}

/// Locate a named definition inside a schemars root schema, whether it is
/// inlined under `properties` or stored under `$defs`/`definitions`.
fn find_definition<'a>(schema: &'a Value, name: &str) -> Option<&'a Value> {
    if let Some(defs) = schema.get("$defs").and_then(Value::as_object)
        && let Some(definition) = defs.get(name)
    {
        return Some(definition);
    }
    if let Some(definitions) = schema.get("definitions").and_then(Value::as_object)
        && let Some(definition) = definitions.get(name)
    {
        return Some(definition);
    }
    let mut stack = vec![schema];
    while let Some(node) = stack.pop() {
        match node {
            Value::Object(map) => {
                if map.get("title").and_then(Value::as_str) == Some(name) {
                    return Some(node);
                }
                stack.extend(map.values());
            }
            Value::Array(items) => stack.extend(items),
            _ => {}
        }
    }
    None
}

#[test]
fn search_request_schema_has_only_intended_fields() {
    let schema = schema_of::<McpSearchRequest>();
    assert_eq!(
        prop_names(&schema),
        vec![
            "cursor",
            "group_path",
            "limit",
            "locale",
            "query",
            "source_key"
        ],
        "MCP search input must not inherit HTTP fields: {schema}"
    );
    for forbidden in [
        "page",
        "metadata_filters",
        "sort",
        "published_after",
        "published_before",
    ] {
        assert!(
            schema
                .get("properties")
                .and_then(|p| p.get(forbidden))
                .is_none(),
            "forbidden HTTP field {forbidden} leaked into MCP search schema"
        );
    }
    let minimum = prop_keywords(&schema, "limit", "minimum");
    let maximum = prop_keywords(&schema, "limit", "maximum");
    assert!(
        minimum.iter().any(|v| v == &json!(1)),
        "limit must declare minimum 1: {schema}"
    );
    assert!(
        maximum.iter().any(|v| v == &json!(20)),
        "limit must declare maximum 20: {schema}"
    );
    let query_max = prop_keywords(&schema, "query", "maxLength");
    assert!(
        query_max.iter().any(|v| v == &json!(2000)),
        "query must declare maxLength 2000: {schema}"
    );
}

#[test]
fn search_request_rejects_unknown_http_fields() {
    let legacy = json!({
        "query": "rates",
        "limit": 8,
        "page": 2,
        "metadata_filters": [],
    });
    let error = serde_json::from_value::<McpSearchRequest>(legacy)
        .expect_err("deprecated page / HTTP filter fields must be rejected, not silently ignored");
    let message = error.to_string();
    assert!(
        message.contains("page") || message.contains("metadata_filters"),
        "unexpected error: {message}"
    );
}

#[test]
fn search_request_defaults_and_validation() {
    let request: McpSearchRequest =
        serde_json::from_value(json!({"query": "rates"})).expect("query-only payload deserializes");
    assert_eq!(request.limit, 8);
    assert!(request.validate().is_ok());

    for payload in [
        json!({"query": "   "}),
        json!({"query": "rates", "limit": 0}),
        json!({"query": "rates", "limit": 21}),
        json!({"query": "rates", "cursor": "  "}),
        json!({"query": "rates", "locale": ""}),
    ] {
        let request: McpSearchRequest =
            serde_json::from_value(payload.clone()).expect("payload shape deserializes");
        assert!(
            request.validate().is_err(),
            "payload must fail validation: {payload}"
        );
    }
}

#[test]
fn search_request_converts_without_http_compat_fields() {
    let request = McpSearchRequest {
        query: "rates".to_string(),
        group_path: Some("research/news".to_string()),
        source_key: Some("news-pg".to_string()),
        locale: Some("zh-CN".to_string()),
        limit: 8,
        cursor: Some("opaque".to_string()),
    };
    let internal = request.to_search_request();
    assert_eq!(internal.query, "rates");
    assert_eq!(internal.limit, 8);
    assert_eq!(internal.page, 1, "deprecated page stays pinned to 1");
    assert!(internal.metadata_filters.is_empty());
    assert!(internal.published_after.is_none());
    assert!(internal.published_before.is_none());
    assert!(matches!(
        internal.sort,
        context69::contracts::SearchSort::Relevance
    ));
    assert_eq!(internal.group_path.as_deref(), Some("research/news"));
    assert_eq!(internal.source_key.as_deref(), Some("news-pg"));
    assert_eq!(internal.locale.as_deref(), Some("zh-CN"));
    assert_eq!(internal.cursor.as_deref(), Some("opaque"));
}

#[test]
fn search_response_continuation_invariant() {
    let terminal = McpSearchResponse::new(Vec::new(), None);
    assert!(!terminal.has_more);
    terminal.validate_continuation().expect("terminal is valid");

    let continued = McpSearchResponse::new(Vec::new(), Some("cursor-1".to_string()));
    assert!(continued.has_more);
    continued
        .validate_continuation()
        .expect("continued is valid");

    let broken = McpSearchResponse {
        hits: Vec::new(),
        next_cursor: None,
        has_more: true,
    };
    assert!(broken.validate_continuation().is_err());
}

#[test]
fn search_hit_schema_declares_snippet_bound() {
    let schema = schema_of::<McpSearchRequest>();
    let _ = schema;
    let hit_schema = schema_of::<context69::contracts::McpSearchHit>();
    let snippet_max = prop_keywords(&hit_schema, "snippet", "maxLength");
    assert!(
        snippet_max.iter().any(|v| v == &json!(600)),
        "snippet must declare maxLength 600: {hit_schema}"
    );
    let response_schema = schema_of::<McpSearchResponse>();
    assert_eq!(
        prop_names(&response_schema),
        vec!["has_more", "hits", "next_cursor"],
        "search response must not carry query/truncated: {response_schema}"
    );
    let hits_max = prop_keywords(&response_schema, "hits", "maxItems");
    assert!(
        hits_max.iter().any(|v| v == &json!(20)),
        "hits must declare maxItems 20: {response_schema}"
    );
}

#[test]
fn search_hit_projection_truncates_snippet() {
    let hit = SearchHit {
        chunk_id: Uuid::new_v4(),
        document_id: 7,
        group_key: "g".to_string(),
        group_path: "research/news".to_string(),
        visibility: Visibility::Public,
        source_key: "news-pg".to_string(),
        external_id: "art-7".to_string(),
        title: "Title".to_string(),
        summary: None,
        source_uri: "postgres://articles/7".to_string(),
        published_at: None,
        chunk_index: 0,
        chunk_text: "y".repeat(5_000),
        score: 0.9,
        vector_score: None,
        keyword_score: None,
        rerank_score: None,
        match_reason: None,
        metadata_json: json!({}),
        library_file_id: None,
        library_section_label: None,
        library_path: None,
        is_library_file: false,
        requested_locale: None,
        content_locale: None,
        translation_status: None,
        is_fallback: false,
    };
    let projected = context69::contracts::McpSearchHit::from_search_hit(&hit);
    assert_eq!(projected.snippet.chars().count(), 600);
}

#[test]
fn source_summary_hides_operational_fields() {
    let summary = McpSourceSummary::from_status(&sample_status(Visibility::Public));
    let value = serde_json::to_value(&summary).expect("summary serializes");
    let object = value.as_object().expect("summary is an object");
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "description",
            "display_name",
            "group_path",
            "origin_status",
            "source_key",
            "visibility"
        ],
        "safe summary must expose exactly the allowlisted fields"
    );
    for forbidden in [
        "connection",
        "has_database_url",
        "origin_message",
        "sync_strategy",
        "connector_type",
        "base_query",
        "batch_size",
        "example_queries",
        "last_cursor_updated_at",
        "last_cursor_external_id",
        "last_success_at",
        "group_key",
    ] {
        assert!(
            !object.contains_key(forbidden),
            "operational field {forbidden} must never cross the MCP boundary"
        );
    }
    assert!(summary.is_public());

    let private = McpSourceSummary::from_status(&sample_status(Visibility::Private));
    assert!(!private.is_public());
}

#[test]
fn source_summary_schema_matches_safe_shape() {
    let schema = schema_of::<McpSourceSummary>();
    assert_eq!(
        prop_names(&schema),
        vec![
            "description",
            "display_name",
            "group_path",
            "origin_status",
            "source_key",
            "visibility"
        ],
        "source summary schema must match the safe shape: {schema}"
    );
}

#[test]
fn source_list_response_shape_and_bounds() {
    let schema = schema_of::<McpSourceListResponse>();
    assert_eq!(
        prop_names(&schema),
        vec!["has_more", "next_cursor", "sources"],
        "source listing must not carry truncated/full statuses: {schema}"
    );
    let sources_max = prop_keywords(&schema, "sources", "maxItems");
    assert!(
        sources_max.iter().any(|v| v == &json!(100)),
        "sources must declare maxItems 100: {schema}"
    );

    let args_schema = schema_of::<McpSourceListArgs>();
    assert_eq!(prop_names(&args_schema), vec!["cursor", "limit"]);
    let limit_max = prop_keywords(&args_schema, "limit", "maximum");
    assert!(limit_max.iter().any(|v| v == &json!(100)));

    let defaults = McpSourceListArgs::default();
    assert_eq!(defaults.limit, 50);
    assert!(defaults.validate().is_ok());
}

#[test]
fn source_cursor_pagination_roundtrip() {
    let summaries: Vec<McpSourceSummary> = (0..120)
        .map(|index| {
            McpSourceSummary::from_status(&SourceStatus {
                source_key: format!("src-{index:03}"),
                ..sample_status(Visibility::Public)
            })
        })
        .collect();

    let first = paginate_source_summaries(
        &summaries,
        &McpSourceListArgs {
            limit: 50,
            cursor: None,
        },
    )
    .expect("first page");
    assert_eq!(first.sources.len(), 50);
    assert!(first.has_more);
    assert_eq!(first.next_cursor.as_deref(), Some("50"));
    first.validate_continuation().expect("valid continuation");

    let cursor = first.next_cursor.clone().unwrap();
    let second = paginate_source_summaries(
        &summaries,
        &McpSourceListArgs {
            limit: 50,
            cursor: Some(cursor),
        },
    )
    .expect("second page");
    assert_eq!(second.sources.len(), 50);
    assert_eq!(second.sources[0].source_key, "src-050");
    assert_eq!(second.next_cursor.as_deref(), Some("100"));

    let cursor = second.next_cursor.clone().unwrap();
    let third = paginate_source_summaries(
        &summaries,
        &McpSourceListArgs {
            limit: 50,
            cursor: Some(cursor),
        },
    )
    .expect("third page");
    assert_eq!(third.sources.len(), 20);
    assert!(!third.has_more);
    assert!(third.next_cursor.is_none());

    let past_end = paginate_source_summaries(
        &summaries,
        &McpSourceListArgs {
            limit: 50,
            cursor: Some("500".to_string()),
        },
    )
    .expect("past-end page is terminal");
    assert!(past_end.sources.is_empty());
    assert!(!past_end.has_more);
}

#[test]
fn source_pagination_rejects_invalid_cursor_and_limit() {
    let summaries = vec![McpSourceSummary::from_status(&sample_status(
        Visibility::Public,
    ))];
    for args in [
        McpSourceListArgs {
            limit: 0,
            cursor: None,
        },
        McpSourceListArgs {
            limit: 101,
            cursor: None,
        },
        McpSourceListArgs {
            limit: 10,
            cursor: Some("not-an-offset".to_string()),
        },
        McpSourceListArgs {
            limit: 10,
            cursor: Some("-3".to_string()),
        },
        McpSourceListArgs {
            limit: 10,
            cursor: Some("  ".to_string()),
        },
    ] {
        assert!(
            args.validate().is_err(),
            "source list args must be rejected: {args:?}"
        );
        assert!(paginate_source_summaries(&summaries, &args).is_err());
    }
    assert!(parse_offset_cursor(None).expect("none") == 0);
    assert!(parse_offset_cursor(Some("12")).expect("offset") == 12);
    assert!(parse_offset_cursor(Some("abc")).is_err());
}

#[test]
fn document_args_validation_and_cursor_parsing() {
    let valid = McpDocumentArgs {
        document_id: 421,
        locale: None,
        chunk_cursor: Some("20".to_string()),
        chunk_limit: 20,
    };
    assert!(valid.validate().is_ok());
    assert_eq!(valid.start_offset().expect("offset"), 20);

    assert_eq!(parse_chunk_cursor(None).expect("default"), 0);
    assert!(parse_chunk_cursor(Some("abc")).is_err());

    for args in [
        McpDocumentArgs {
            document_id: 0,
            locale: None,
            chunk_cursor: None,
            chunk_limit: 20,
        },
        McpDocumentArgs {
            document_id: 1,
            locale: None,
            chunk_cursor: None,
            chunk_limit: 0,
        },
        McpDocumentArgs {
            document_id: 1,
            locale: None,
            chunk_cursor: None,
            chunk_limit: 51,
        },
        McpDocumentArgs {
            document_id: 1,
            locale: None,
            chunk_cursor: Some("NaN".to_string()),
            chunk_limit: 20,
        },
    ] {
        assert!(
            args.validate().is_err(),
            "document args must be rejected: {args:?}"
        );
    }
}

#[test]
fn document_detail_window_matches_schema_caps() {
    let document = sample_document(7, 5_000);
    let first = paginate_document_detail(&document, 0, 5).expect("first window");
    assert_eq!(first.document.chunks.len(), 5);
    assert!(first.has_more);
    assert_eq!(first.next_chunk_cursor.as_deref(), Some("5"));
    first.validate_continuation().expect("valid continuation");
    for chunk in &first.document.chunks {
        assert_eq!(chunk.text.chars().count(), 4_000);
    }
    assert_eq!(first.document.chunks[0].index, 0);

    let rest = paginate_document_detail(&document, 5, 5).expect("second window");
    assert_eq!(rest.document.chunks.len(), 2);
    assert!(!rest.has_more);
    assert!(rest.next_chunk_cursor.is_none());
    assert_eq!(rest.document.chunks[0].index, 5);

    // Header carries no operational fields.
    let header = serde_json::to_value(&rest.document).expect("detail serializes");
    for forbidden in [
        "record_hash",
        "metadata_json",
        "group_key",
        "chunks_truncated",
    ] {
        assert!(
            header.get(forbidden).is_none(),
            "operational field {forbidden} leaked into MCP detail"
        );
    }
    assert!(paginate_document_detail(&document, 0, 0).is_err());
    assert!(paginate_document_detail(&document, 0, 51).is_err());
}

#[test]
fn document_detail_schema_declares_chunk_bounds() {
    let schema = schema_of::<McpDocumentDetailResponse>();
    assert_eq!(
        prop_names(&schema),
        vec!["document", "has_more", "next_chunk_cursor"],
        "detail response must not embed DocumentResponse/truncated: {schema}"
    );
    let chunk_schema = schema_of::<context69::contracts::McpDocumentChunk>();
    let text_max = prop_keywords(&chunk_schema, "text", "maxLength");
    assert!(
        text_max.iter().any(|v| v == &json!(4000)),
        "chunk text must declare maxLength 4000: {chunk_schema}"
    );
}

#[test]
fn query_and_batch_shapes_reject_invalid_limits() {
    let too_big = McpDocumentQueryArgs {
        group_path: "research/news".to_string(),
        query: sample_query_request(21),
    };
    assert!(too_big.validate().is_err());
    let zero = McpDocumentQueryArgs {
        group_path: "research/news".to_string(),
        query: sample_query_request(0),
    };
    assert!(zero.validate().is_err());
    let ok = McpDocumentQueryArgs {
        group_path: "research/news".to_string(),
        query: sample_query_request(20),
    };
    assert!(ok.validate().is_ok());

    let query_schema = schema_of::<McpDocumentQueryArgs>();
    assert_eq!(prop_names(&query_schema), vec!["group_path", "query"]);
    // No schema definition or $ref may target the HTTP request DTO; prose
    // descriptions may still name the internal conversion target.
    let mut referenced = Vec::new();
    collect_schema_refs(&query_schema, &mut referenced);
    assert!(
        referenced
            .iter()
            .all(|name| !name.contains("DocumentQueryRequest")),
        "tool input must not embed the HTTP request DTO: {referenced:?}"
    );
    let nested = find_definition(&query_schema, "McpDocumentQuery")
        .expect("nested MCP query definition is discoverable");
    assert_eq!(
        prop_names(nested),
        vec![
            "cursor",
            "limit",
            "locale",
            "metadata_filters",
            "published_after",
            "published_before",
            "sort",
            "source_key"
        ],
        "nested query exposes exactly the useful MCP filters: {nested}"
    );
    let nested_limit_default = prop_keywords(nested, "limit", "default");
    assert!(
        nested_limit_default.iter().any(|v| v == &json!(20)),
        "nested limit must declare default 20: {nested}"
    );
    let nested_limit_min = prop_keywords(nested, "limit", "minimum");
    let nested_limit_max = prop_keywords(nested, "limit", "maximum");
    assert!(
        nested_limit_min.iter().any(|v| v == &json!(1)),
        "nested limit must declare minimum 1: {nested}"
    );
    assert!(
        nested_limit_max.iter().any(|v| v == &json!(20)),
        "nested limit must declare maximum 20: {nested}"
    );

    let response_schema = schema_of::<context69::contracts::McpDocumentQueryResponse>();
    assert_eq!(
        prop_names(&response_schema),
        vec!["documents", "has_more", "next_cursor"],
        "query response must not carry truncated: {response_schema}"
    );

    let empty_keys = McpBatchDocumentArgs {
        group_path: "research/news".to_string(),
        request: McpBatchDocumentKeys {
            keys: Vec::new(),
            locale: None,
        },
    };
    assert!(empty_keys.validate().is_err());
    let too_many = McpBatchDocumentArgs {
        group_path: "research/news".to_string(),
        request: McpBatchDocumentKeys {
            keys: (0..21)
                .map(|index| DocumentKey {
                    source_key: "news-pg".to_string(),
                    external_id: format!("art-{index}"),
                })
                .collect(),
            locale: None,
        },
    };
    assert!(too_many.validate().is_err());

    let batch_schema = schema_of::<context69::contracts::McpBatchDocumentResponse>();
    assert_eq!(prop_names(&batch_schema), vec!["has_more", "items"]);
    let items_max = prop_keywords(&batch_schema, "items", "maxItems");
    assert!(items_max.iter().any(|v| v == &json!(20)));
}

#[test]
fn query_omitted_limit_uses_mcp_default() {
    // Regression: the natural call without a nested limit must not inherit the
    // HTTP DTO default (50) and fail MCP validation.
    let omitted: McpDocumentQueryArgs = serde_json::from_value(json!({
        "group_path": "research/news",
        "query": {"source_key": "news-pg"},
    }))
    .expect("omitted nested limit deserializes");
    assert_eq!(
        omitted.query.limit,
        context69::contracts::MCP_QUERY_LIMIT_DEFAULT,
        "MCP-local default must apply when limit is omitted"
    );
    assert_eq!(omitted.query.limit, 20);
    assert!(omitted.validate().is_ok());

    // An explicitly supplied limit still flows through unchanged.
    let explicit: McpDocumentQueryArgs = serde_json::from_value(json!({
        "group_path": "research/news",
        "query": {"source_key": "news-pg", "limit": 5},
    }))
    .expect("explicit nested limit deserializes");
    assert_eq!(explicit.query.limit, 5);
    assert!(explicit.validate().is_ok());

    // An explicitly oversized limit is still rejected at validation.
    let oversized: McpDocumentQueryArgs = serde_json::from_value(json!({
        "group_path": "research/news",
        "query": {"source_key": "news-pg", "limit": 50},
    }))
    .expect("explicit oversized limit deserializes");
    assert!(oversized.validate().is_err());
}

#[test]
fn query_conversion_preserves_filters_and_normalizes_window() {
    let args: McpDocumentQueryArgs = serde_json::from_value(json!({
        "group_path": "research/news",
        "query": {
            "locale": "zh-CN",
            "source_key": "news-pg",
            "metadata_filters": [{"path": "desk", "operator": "eq", "value": "markets"}],
            "limit": 7,
            "cursor": "opaque",
        },
    }))
    .expect("filtered query deserializes");
    assert!(args.validate().is_ok());
    let internal = args.query.to_document_query_request();
    assert_eq!(internal.locale.as_deref(), Some("zh-CN"));
    assert_eq!(internal.source_key.as_deref(), Some("news-pg"));
    assert_eq!(internal.metadata_filters.len(), 1);
    assert_eq!(internal.metadata_filters[0].path, "desk");
    assert_eq!(internal.limit, 7);
    assert_eq!(internal.cursor.as_deref(), Some("opaque"));
}

#[test]
fn batch_keys_shape_declares_max_items_and_rejects_over_limit() {
    let keys_schema = schema_of::<McpBatchDocumentKeys>();
    assert_eq!(prop_names(&keys_schema), vec!["keys", "locale"]);
    let keys_max = prop_keywords(&keys_schema, "keys", "maxItems");
    assert!(
        keys_max.iter().any(|v| v == &json!(20)),
        "batch keys must declare maxItems 20: {keys_schema}"
    );
    let keys_min = prop_keywords(&keys_schema, "keys", "minItems");
    assert!(
        keys_min.iter().any(|v| v == &json!(1)),
        "batch keys must declare minItems 1 to match empty rejection: {keys_schema}"
    );

    let keys_value = |count: usize| {
        (0..count)
            .map(|index| json!({"source_key": "news-pg", "external_id": format!("art-{index}")}))
            .collect::<Vec<_>>()
    };
    // Oversized lists are rejected at deserialization, before validation runs.
    let error = serde_json::from_value::<McpBatchDocumentKeys>(json!({
        "keys": keys_value(21),
    }))
    .expect_err("21 keys must be rejected at deserialization");
    assert!(
        error.to_string().contains("1..=20"),
        "unexpected error: {error}"
    );

    let ok: McpBatchDocumentKeys = serde_json::from_value(json!({
        "keys": keys_value(20),
    }))
    .expect("20 keys deserialize");
    assert!(ok.validate().is_ok());

    let empty: McpBatchDocumentKeys =
        serde_json::from_value(json!({"keys": []})).expect("empty keys deserialize");
    assert!(empty.validate().is_err());
}

#[test]
fn key_args_validation() {
    let valid = McpDocumentKeyArgs {
        group_path: "research/news".to_string(),
        key: DocumentKey {
            source_key: "news-pg".to_string(),
            external_id: "art-1".to_string(),
        },
        locale: None,
    };
    assert!(valid.validate().is_ok());
    let blank = McpDocumentKeyArgs {
        group_path: "  ".to_string(),
        key: DocumentKey {
            source_key: "news-pg".to_string(),
            external_id: "art-1".to_string(),
        },
        locale: None,
    };
    assert!(blank.validate().is_err());
}

#[test]
fn truncate_chars_is_boundary_safe() {
    assert_eq!(
        context69::contracts::truncate_chars("héllo世界", 5),
        "héllo"
    );
    assert_eq!(context69::contracts::truncate_chars("abc", 5), "abc");
}
