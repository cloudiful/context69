use context69_contracts::{
    ApiErrorCode, CanonicalApiErrorResponse, CanonicalSearchRequest, CanonicalTaskListQuery,
    CanonicalUpdateSearchSettingsRequest, CanonicalUploadMetadata, CreateTextRequest,
    CursorPageQuery, CursorPagination, FileBatchItem, ImportLibraryFileFromUrlRequest,
    IngestOptions, LibraryFileIngestOptions, LibraryTextContentFormat, MetadataObject,
    OffsetPageQuery, OffsetPagination, Pagination, PersonalAccessTokenScope,
    PrepareLibraryUploadRequest, SearchRequest, SecretPatch, SortDirection, SortOrder,
    SourcePolicy, TaskListQuery, TaskListView, UpsertLibraryTextRequest,
};
use serde_json::{json, to_value};

#[test]
fn legacy_requests_receive_current_defaults() {
    let text: CreateTextRequest = serde_json::from_value(json!({
        "title": "Plain Doc",
        "content": "Hello"
    }))
    .expect("legacy text request");
    assert_eq!(text.content_format, LibraryTextContentFormat::PlainText);

    let upsert: UpsertLibraryTextRequest = serde_json::from_value(json!({
        "external_id": "doc-1",
        "title": "Doc",
        "content": "Hello",
        "published_at": "2026-06-10T08:30:00+08:00"
    }))
    .expect("legacy upsert request");
    assert_eq!(upsert.metadata_json, json!({}));
    assert_eq!(upsert.content_format, LibraryTextContentFormat::PlainText);
    assert_eq!(
        upsert.published_at.expect("published_at").to_rfc3339(),
        "2026-06-10T00:30:00+00:00"
    );

    let import: ImportLibraryFileFromUrlRequest = serde_json::from_value(json!({
        "url": "https://files.example.test/report.pdf"
    }))
    .expect("legacy URL import request");
    assert!(import.metadata.is_none());
    assert!(import.translation.is_none());
}

#[test]
fn access_token_scope_wire_names_are_stable() {
    assert_eq!(
        to_value([
            PersonalAccessTokenScope::Search,
            PersonalAccessTokenScope::Library,
            PersonalAccessTokenScope::Admin,
        ])
        .expect("serialize scopes"),
        json!(["search", "library", "admin"])
    );
}

#[test]
fn pagination_window_signals_are_optional_and_backward_compatible() {
    // Legacy payloads without the new keys keep exact-total semantics.
    let legacy: Pagination = serde_json::from_value(json!({
        "page": 1,
        "page_size": 8,
        "total": 20,
        "total_pages": 3
    }))
    .expect("legacy pagination");
    assert_eq!(legacy.has_more, None);
    assert_eq!(legacy.total_is_exact, None);

    // Fresh responses omit the keys unless a window signal is set.
    let exact = Pagination::try_new(1, 8, 20).expect("exact pagination");
    let exact_value = to_value(exact).expect("serialize exact");
    assert!(
        !exact_value
            .as_object()
            .expect("object")
            .contains_key("has_more")
    );
    assert!(
        !exact_value
            .as_object()
            .expect("object")
            .contains_key("total_is_exact")
    );

    // Search windows explicitly mark the lower bound and probe result.
    let window = Pagination::try_new_search_window(1, 8, 9, Some(true)).expect("search window");
    assert_eq!(window.has_more, Some(true));
    assert_eq!(window.total_is_exact, Some(false));
    let window_value = to_value(window).expect("serialize window");
    assert_eq!(window_value.get("has_more"), Some(&json!(true)));
    assert_eq!(window_value.get("total_is_exact"), Some(&json!(false)));

    // Unknown probe state round-trips without claiming end-of-results.
    let capped = Pagination::try_new_search_window(5, 8, 2_000, None).expect("capped window");
    let capped_value = to_value(capped).expect("serialize capped");
    assert!(
        !capped_value
            .as_object()
            .expect("object")
            .contains_key("has_more")
    );
    assert_eq!(capped_value.get("total_is_exact"), Some(&json!(false)));
    let decoded: Pagination = serde_json::from_value(capped_value).expect("deserialize capped");
    assert_eq!(decoded.has_more, None);
    assert_eq!(decoded.total_is_exact, Some(false));
}

const INVENTORY: &str = include_str!("../../../docs/contracts/v0.16-inventory.md");

fn inventory_http_rows() -> Vec<Vec<String>> {
    let marker = "## HTTP operations";
    let start = INVENTORY
        .find(marker)
        .expect("inventory has ## HTTP operations");
    let rest = &INVENTORY[start..];
    let end = rest[marker.len()..]
        .find("\n## ")
        .map(|i| i + marker.len())
        .unwrap_or(rest.len());
    let section = &rest[..end];
    let mut rows = Vec::new();
    for line in section.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with('|') {
            continue;
        }
        let raw: Vec<&str> = trimmed.split('|').collect();
        if raw.len() < 11 {
            continue;
        }
        let inner: Vec<String> = raw[1..raw.len() - 1]
            .iter()
            .map(|s| s.trim().to_string())
            .collect();
        if inner.len() != 9 || inner[0] == "operation_id" || inner[0].starts_with("---") {
            continue;
        }
        rows.push(inner);
    }
    rows
}

fn inventory_shared_schemas() -> Vec<String> {
    let marker = "## Shared schemas";
    let start = INVENTORY
        .find(marker)
        .expect("inventory has ## Shared schemas");
    let rest = &INVENTORY[start..];
    let end = rest[marker.len()..]
        .find("\n## ")
        .map(|i| i + marker.len())
        .unwrap_or(rest.len());
    let section = &rest[..end];
    let mut out = Vec::new();
    for line in section.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("- `") {
            continue;
        }
        let begin = trimmed.find('`').expect("backtick") + 1;
        let finish = trimmed[begin..].find('`').expect("closing") + begin;
        let name = trimmed[begin..finish].trim().to_string();
        if !name.is_empty() {
            out.push(name);
        }
    }
    out
}

#[test]
fn contract_inventory_is_machine_checkable_and_frozen() {
    let rows = inventory_http_rows();
    assert!(!rows.is_empty(), "inventory must list HTTP operations");
    let mut ids = std::collections::HashSet::new();
    for row in &rows {
        assert_eq!(row.len(), 9, "each inventory row has 9 columns: {row:?}");
        assert!(
            ids.insert(row[0].clone()),
            "duplicate operation_id in inventory: {}",
            row[0]
        );
        assert!(
            ["GET", "POST", "PUT", "PATCH", "DELETE"].contains(&row[1].as_str()),
            "bad method {} for {}",
            row[1],
            row[0]
        );
        assert!(
            ["high-level", "none", "raw"].contains(&row[6].as_str()),
            "bad sdk_surface {} for {}",
            row[6],
            row[0]
        );
        assert!(
            ["public", "authenticated", "admin"].contains(&row[8].as_str()),
            "bad visibility {} for {}",
            row[8],
            row[0]
        );
    }
    let shared = inventory_shared_schemas();
    assert!(!shared.is_empty(), "inventory must list shared schemas");
    let mut shared_seen = std::collections::HashSet::new();
    for name in &shared {
        assert!(
            shared_seen.insert(name.clone()),
            "duplicate shared schema: {name}"
        );
    }
    for required in [
        "ApiErrorResponse",
        "SearchRequest",
        "SearchResponse",
        "TaskListQuery",
        "TaskPageResponse",
        "Pagination",
        "SourceStatus",
        "DocumentResponse",
    ] {
        assert!(
            shared_seen.contains(required),
            "inventory shared schemas must contain {required}"
        );
    }
    // SDK and MCP surfaces are enumerated in prose sections.
    for method in ["task_items", "search_compact", "release_file_source", "me"] {
        assert!(
            INVENTORY.contains(&format!("`{method}`")),
            "inventory must mention SDK method `{method}`"
        );
    }
    for tool in ["search_documents", "list_sources"] {
        assert!(
            INVENTORY.contains(tool),
            "inventory must mention MCP tool {tool}"
        );
    }
}

#[test]
fn canonical_offset_query_defaults_and_rejects_zero() {
    let empty: OffsetPageQuery = serde_json::from_value(json!({})).expect("offset query defaults");
    assert_eq!(empty.page, 1);
    assert_eq!(empty.page_size, 50);
    empty.validate().expect("defaults validate");

    let explicit: OffsetPageQuery = serde_json::from_value(json!({
        "page": 3,
        "page_size": 25
    }))
    .expect("explicit offset query");
    assert_eq!(explicit.offset().expect("offset"), 50);

    let zero_page: OffsetPageQuery = serde_json::from_value(json!({
        "page": 0,
        "page_size": 25
    }))
    .expect("zero page parses, validation rejects");
    assert!(zero_page.validate().is_err());
    assert!(OffsetPagination::try_new(0, 25, 10).is_err());
    assert!(OffsetPagination::try_new(1, 0, 10).is_err());
    assert!(OffsetPagination::try_new(1, 101, 10).is_err());

    let exact = OffsetPagination::try_new(1, 8, 20).expect("offset pagination");
    assert_eq!(exact.total_pages, 3);
    let empty_total = OffsetPagination::try_new(1, 8, 0).expect("empty total");
    assert_eq!(empty_total.total_pages, 0);
}

#[test]
fn canonical_cursor_query_defaults_and_continuation() {
    let empty: CursorPageQuery = serde_json::from_value(json!({})).expect("cursor query defaults");
    assert_eq!(empty.limit, 50);
    assert_eq!(empty.cursor, None);
    empty.validate().expect("defaults validate");
    let serialized = to_value(&empty).expect("serialize cursor query");
    assert!(
        !serialized
            .as_object()
            .expect("object")
            .contains_key("cursor"),
        "cursor omits None"
    );

    let zero: CursorPageQuery = serde_json::from_value(json!({ "limit": 0 }))
        .expect("zero limit parses, validation rejects");
    assert!(zero.validate().is_err());

    let page = CursorPagination::new(Some("tok-1".to_string()), true);
    page.validate_continuation()
        .expect("has_more with token validates");
    let terminal = CursorPagination::terminal();
    assert!(terminal.is_terminal());
    assert!(!terminal.has_more);
    assert_eq!(terminal.next_cursor, None);

    let dangling = CursorPagination::new(None, true);
    assert!(dangling.validate_continuation().is_err());

    let next: CursorPageQuery = serde_json::from_value(json!({
        "limit": 8,
        "cursor": "tok-1"
    }))
    .expect("cursor continuation");
    assert_eq!(next.cursor.as_deref(), Some("tok-1"));
}

#[test]
fn canonical_pagination_schemas_carry_min_max() {
    for (name, schema) in [
        ("OffsetPageQuery", schemars::schema_for!(OffsetPageQuery)),
        ("CursorPageQuery", schemars::schema_for!(CursorPageQuery)),
        (
            "CanonicalSearchRequest",
            schemars::schema_for!(CanonicalSearchRequest),
        ),
        (
            "CanonicalTaskListQuery",
            schemars::schema_for!(CanonicalTaskListQuery),
        ),
    ] {
        let value = to_value(&schema).expect("schema to value");
        let text = serde_json::to_string(&value).expect("schema to string");
        assert!(
            text.contains("\"minimum\""),
            "{name} schema must carry minimum"
        );
        assert!(
            text.contains("\"maximum\""),
            "{name} schema must carry maximum"
        );
    }

    let offset_schema = to_value(schemars::schema_for!(OffsetPageQuery)).expect("offset schema");
    let offset_text = serde_json::to_string(&offset_schema).expect("offset text");
    assert!(
        offset_text.contains("10000"),
        "offset page max 10000 must appear in schema"
    );
}

#[test]
fn sort_direction_is_single_and_wire_stable() {
    assert_eq!(to_value(SortDirection::Asc).expect("asc"), json!("asc"));
    assert_eq!(to_value(SortDirection::Desc).expect("desc"), json!("desc"));
    assert_eq!(SortDirection::Asc.as_str(), "asc");
    assert_eq!(SortDirection::Desc.as_str(), "desc");
    assert_eq!(
        "asc".parse::<SortDirection>().expect("parse asc"),
        SortDirection::Asc
    );
    assert_eq!(SortDirection::default(), SortDirection::Asc);

    assert_eq!(to_value(SortOrder::Asc).expect("order asc"), json!("asc"));
    assert_eq!(
        to_value(SortOrder::Desc).expect("order desc"),
        json!("desc")
    );
    assert_eq!(SortDirection::from(SortOrder::Asc), SortDirection::Asc);
    assert_eq!(SortOrder::from(SortDirection::Desc), SortOrder::Desc);

    let legacy_sort = context69_contracts::DocumentSort {
        field: context69_contracts::DocumentSortField::PublishedAt,
        order: SortOrder::Desc,
    };
    let canonical = context69_contracts::CanonicalDocumentSort::from(legacy_sort.clone());
    assert_eq!(canonical.direction, SortDirection::Desc);
    let round_trip = context69_contracts::DocumentSort::from(canonical);
    assert_eq!(round_trip.order, legacy_sort.order);
}

#[test]
fn api_error_code_wire_names_and_typed_shape() {
    for (code, wire) in [
        (ApiErrorCode::InvalidArgument, "invalid_argument"),
        (ApiErrorCode::Unauthorized, "unauthorized"),
        (ApiErrorCode::Forbidden, "forbidden"),
        (ApiErrorCode::NotFound, "not_found"),
        (ApiErrorCode::Conflict, "conflict"),
        (ApiErrorCode::PayloadTooLarge, "payload_too_large"),
        (ApiErrorCode::UnprocessableEntity, "unprocessable_entity"),
        (ApiErrorCode::RateLimited, "rate_limited"),
        (ApiErrorCode::UpstreamError, "upstream_error"),
        (ApiErrorCode::Unavailable, "unavailable"),
        (ApiErrorCode::UpstreamTimeout, "upstream_timeout"),
        (ApiErrorCode::Internal, "internal"),
    ] {
        assert_eq!(to_value(code).expect("code wire"), json!(wire));
        assert_eq!(code.as_str(), wire);
        assert_eq!(wire.parse::<ApiErrorCode>().expect("parse code"), code);
    }
    assert_eq!(
        ApiErrorCode::code_for_status(400),
        ApiErrorCode::InvalidArgument
    );
    assert_eq!(ApiErrorCode::code_for_status(404), ApiErrorCode::NotFound);
    assert_eq!(
        ApiErrorCode::code_for_status(503),
        ApiErrorCode::Unavailable
    );
    assert_eq!(ApiErrorCode::code_for_status(599), ApiErrorCode::Internal);
    assert_eq!(ApiErrorCode::NotFound.status_code(), 404);

    let minimal = CanonicalApiErrorResponse::new(ApiErrorCode::NotFound, "missing".to_string());
    let minimal_value = to_value(&minimal).expect("minimal error");
    assert_eq!(minimal_value.get("code"), Some(&json!("not_found")));
    assert!(
        !minimal_value
            .as_object()
            .expect("object")
            .contains_key("details")
    );
    assert!(
        !minimal_value
            .as_object()
            .expect("object")
            .contains_key("request_id")
    );

    let full = minimal
        .with_details(json!({ "field": "query" }))
        .with_request_id("req-1".to_string());
    let full_value = to_value(&full).expect("full error");
    assert_eq!(full_value.get("request_id"), Some(&json!("req-1")));

    let legacy_old: context69_contracts::ApiErrorResponse =
        serde_json::from_value(json!({ "code": "not_found", "message": "missing" }))
            .expect("legacy error without request_id");
    assert_eq!(legacy_old.request_id, None);
    assert_eq!(
        legacy_old.code_enum().expect("code enum"),
        ApiErrorCode::NotFound
    );

    let legacy_new: context69_contracts::ApiErrorResponse = serde_json::from_value(json!({
        "code": "not_found",
        "message": "missing",
        "request_id": "req-1"
    }))
    .expect("legacy error with request_id");
    assert_eq!(legacy_new.request_id.as_deref(), Some("req-1"));

    let converted: context69_contracts::ApiErrorResponse = full.into();
    assert_eq!(converted.code, "not_found");
    assert_eq!(converted.request_id.as_deref(), Some("req-1"));
}

#[test]
fn source_policy_and_ingest_options_cover_all_upload_paths() {
    assert_eq!(
        to_value(SourcePolicy::Retain).expect("retain"),
        json!("retain")
    );
    assert_eq!(
        to_value(SourcePolicy::ReleaseAfterProcessing).expect("release"),
        json!("release_after_processing")
    );
    assert_eq!(SourcePolicy::default(), SourcePolicy::Retain);
    assert_eq!(SourcePolicy::from_delete_flag(false), SourcePolicy::Retain);
    assert_eq!(
        SourcePolicy::from_delete_flag(true),
        SourcePolicy::ReleaseAfterProcessing
    );

    let empty: IngestOptions = serde_json::from_value(json!({})).expect("ingest options default");
    assert_eq!(empty.source_policy, SourcePolicy::Retain);
    assert!(empty.translation.is_none());
    assert!(empty.extraction.is_none());
    assert!(!empty.is_release());
    let serialized = to_value(&empty).expect("serialize ingest");
    assert!(
        !serialized
            .as_object()
            .expect("object")
            .contains_key("translation")
    );

    let release = IngestOptions::from_legacy(None, None, None, true);
    assert!(release.is_release());
    assert!(release.as_delete_flag());

    let upload = LibraryFileIngestOptions {
        metadata: Default::default(),
        translation: None,
        extraction: None,
        delete_source_after_processing: true,
    };
    assert!(upload.ingest_options().is_release());

    let prepare: PrepareLibraryUploadRequest = serde_json::from_value(json!({
        "filename": "a.pdf",
        "media_type": "application/pdf",
        "size_bytes": 10,
        "sha256": "abc"
    }))
    .expect("prepare legacy");
    assert!(!prepare.ingest_options().is_release());

    let import: ImportLibraryFileFromUrlRequest = serde_json::from_value(json!({
        "url": "https://files.example.test/report.pdf"
    }))
    .expect("import legacy");
    assert!(!import.ingest_options().is_release());

    let batch: FileBatchItem = serde_json::from_value(json!({
        "filename": "a.pdf",
        "media_type": "application/pdf",
        "content_base64": "aGk=",
        "delete_source_after_processing": true
    }))
    .expect("batch legacy");
    assert!(batch.ingest_options().is_release());
}

#[test]
fn canonical_ingest_options_wire_prefers_options_and_keeps_legacy_readable() {
    // Legacy v0.15 payloads without `options` stay readable for in-flight tasks.
    let legacy_prepare: PrepareLibraryUploadRequest = serde_json::from_value(json!({
        "filename": "a.pdf",
        "media_type": "application/pdf",
        "size_bytes": 10,
        "sha256": "abc",
        "metadata": { "external_id": "doc-1", "metadata_json": { "agency": "x" } },
        "delete_source_after_processing": true
    }))
    .expect("legacy prepare");
    assert!(legacy_prepare.options.is_none());
    let legacy_ingest = legacy_prepare.ingest_options();
    assert!(legacy_ingest.is_release());
    assert_eq!(legacy_ingest.metadata.external_id.as_deref(), Some("doc-1"));

    // Canonical v0.16 payload with `options` takes precedence over flattened
    // duplicates, even when they disagree.
    let canonical_prepare: PrepareLibraryUploadRequest = serde_json::from_value(json!({
        "filename": "a.pdf",
        "media_type": "application/pdf",
        "size_bytes": 10,
        "sha256": "abc",
        "options": {
            "metadata": { "external_id": "doc-2", "metadata_json": {} },
            "source_policy": "retain"
        },
        "metadata": { "external_id": "doc-1", "metadata_json": {} },
        "delete_source_after_processing": true
    }))
    .expect("canonical prepare");
    let canonical_ingest = canonical_prepare.ingest_options();
    assert!(!canonical_ingest.is_release());
    assert_eq!(
        canonical_ingest.metadata.external_id.as_deref(),
        Some("doc-2")
    );

    // Dual-write helper carries both shapes consistently.
    let base: PrepareLibraryUploadRequest = serde_json::from_value(json!({
        "filename": "a.pdf",
        "media_type": "application/pdf",
        "size_bytes": 10,
        "sha256": "abc"
    }))
    .expect("base prepare");
    let dual = base.with_ingest_options(IngestOptions::from_legacy(
        Some(context69_contracts::LibraryFileUploadMetadata {
            external_id: Some("doc-3".to_string()),
            source_uri: None,
            published_at: None,
            metadata_json: json!({ "k": "v" }),
        }),
        None,
        None,
        true,
    ));
    assert!(dual.options.as_ref().expect("options").is_release());
    assert!(dual.delete_source_after_processing);
    assert_eq!(
        dual.metadata
            .as_ref()
            .and_then(|value| value.external_id.clone()),
        Some("doc-3".to_string())
    );
    let round_trip: PrepareLibraryUploadRequest =
        serde_json::from_value(to_value(&dual).expect("serialize dual")).expect("dual round-trips");
    assert!(round_trip.ingest_options().is_release());

    // URL import mirrors the same precedence.
    let legacy_import: ImportLibraryFileFromUrlRequest = serde_json::from_value(json!({
        "url": "https://files.example.test/report.pdf",
        "delete_source_after_processing": true
    }))
    .expect("legacy import");
    assert!(legacy_import.ingest_options().is_release());
    let canonical_import: ImportLibraryFileFromUrlRequest = serde_json::from_value(json!({
        "url": "https://files.example.test/report.pdf",
        "options": { "metadata": {}, "source_policy": "release_after_processing" },
        "delete_source_after_processing": false
    }))
    .expect("canonical import");
    assert!(canonical_import.ingest_options().is_release());

    // File batch mirrors the same precedence and stays readable without options.
    let legacy_batch: FileBatchItem = serde_json::from_value(json!({
        "filename": "a.pdf",
        "media_type": "application/pdf",
        "content_base64": "aGk=",
        "metadata": { "metadata_json": { "k": "v" } },
        "delete_source_after_processing": false
    }))
    .expect("legacy batch");
    assert!(!legacy_batch.ingest_options().is_release());
    let canonical_batch: FileBatchItem = serde_json::from_value(json!({
        "filename": "a.pdf",
        "media_type": "application/pdf",
        "content_base64": "aGk=",
        "options": { "metadata": {}, "source_policy": "release_after_processing" },
        "delete_source_after_processing": false
    }))
    .expect("canonical batch");
    assert!(canonical_batch.ingest_options().is_release());
    let dual_batch = FileBatchItem {
        filename: "a.pdf".to_string(),
        media_type: "application/pdf".to_string(),
        content_base64: "aGk=".to_string(),
        declared_sha256: None,
        folder_id: None,
        options: None,
        metadata: None,
        translation: None,
        extraction: None,
        delete_source_after_processing: false,
    }
    .with_ingest_options(IngestOptions::from_legacy(None, None, None, true));
    assert!(dual_batch.options.as_ref().expect("options").is_release());
    assert!(dual_batch.delete_source_after_processing);

    // Empty canonical metadata maps to no legacy metadata.
    let empty = IngestOptions::default();
    assert!(empty.legacy_metadata_opt().is_none());
    let filled = IngestOptions::from_legacy(
        Some(context69_contracts::LibraryFileUploadMetadata {
            external_id: Some("doc-1".to_string()),
            source_uri: None,
            published_at: None,
            metadata_json: json!({}),
        }),
        None,
        None,
        false,
    );
    assert_eq!(
        filled
            .legacy_metadata_opt()
            .and_then(|value| value.external_id),
        Some("doc-1".to_string())
    );
}

#[test]
fn canonical_search_request_is_cursor_only() {
    let minimal: CanonicalSearchRequest = serde_json::from_value(json!({
        "query": "hello"
    }))
    .expect("canonical search defaults");
    assert_eq!(minimal.limit, 8);
    assert_eq!(minimal.cursor, None);
    assert_eq!(minimal.sort, context69_contracts::SearchSort::Relevance);
    minimal.validate().expect("minimal validates");
    let legacy_default: SearchRequest = serde_json::from_value(json!({
        "query": "hello"
    }))
    .expect("legacy search defaults");
    assert_eq!(
        minimal.limit as usize, legacy_default.limit,
        "canonical default limit must match the v0.15 SearchRequest wire default"
    );

    assert!(serde_json::from_value::<CanonicalSearchRequest>(json!({})).is_err());
    let blank: CanonicalSearchRequest = serde_json::from_value(json!({
        "query": "   ",
        "limit": 8
    }))
    .expect("blank parses, validation rejects");
    assert!(blank.validate().is_err());
    let zero: CanonicalSearchRequest = serde_json::from_value(json!({
        "query": "hello",
        "limit": 0
    }))
    .expect("zero parses, validation rejects");
    assert!(zero.validate().is_err());

    let continued = minimal.clone().with_cursor(Some("tok-1".to_string()));
    assert_eq!(continued.cursor.as_deref(), Some("tok-1"));

    let serialized = to_value(&minimal).expect("serialize canonical search");
    assert!(
        !serialized.as_object().expect("object").contains_key("page"),
        "canonical search must not carry legacy page"
    );

    let legacy: SearchRequest = continued.clone().into();
    assert_eq!(legacy.page, 1);
    assert_eq!(legacy.cursor.as_deref(), Some("tok-1"));
    let legacy_minimal: SearchRequest = minimal.into();
    assert_eq!(legacy_minimal.page, 1);
    assert_eq!(legacy_minimal.cursor, None);
}

#[test]
fn canonical_task_list_query_requires_view() {
    assert!(
        serde_json::from_value::<CanonicalTaskListQuery>(json!({
            "page": 1,
            "page_size": 25
        }))
        .is_err()
    );

    let canonical: CanonicalTaskListQuery = serde_json::from_value(json!({
        "page": 1,
        "page_size": 25,
        "view": "processing"
    }))
    .expect("canonical task query");
    assert_eq!(canonical.view, TaskListView::Processing);
    canonical.validate().expect("canonical validates");

    let serialized = to_value(&canonical).expect("serialize canonical task");
    assert!(
        !serialized
            .as_object()
            .expect("object")
            .contains_key("trashed"),
        "canonical task must not carry trashed"
    );

    let zero: CanonicalTaskListQuery = serde_json::from_value(json!({
        "page": 0,
        "page_size": 25,
        "view": "processing"
    }))
    .expect("zero parses, validation rejects");
    assert!(zero.validate().is_err());

    let legacy: TaskListQuery = canonical.into();
    assert_eq!(legacy.view, Some(TaskListView::Processing));
    assert_eq!(legacy.trashed, None);

    let legacy_default: TaskListQuery = serde_json::from_value(json!({
        "page": 1,
        "page_size": 25
    }))
    .expect("legacy without view");
    assert_eq!(legacy_default.view, None);
}

#[test]
fn metadata_object_is_explicit_without_breaking_legacy() {
    let empty: MetadataObject = serde_json::from_value(json!({})).expect("empty object");
    assert!(empty.is_empty());

    let filled: MetadataObject = serde_json::from_value(json!({ "a": 1 })).expect("object");
    assert_eq!(filled.get("a"), Some(&json!(1)));

    assert!(serde_json::from_value::<MetadataObject>(json!([])).is_err());
    assert!(serde_json::from_value::<MetadataObject>(json!("x")).is_err());
    assert!(serde_json::from_value::<MetadataObject>(json!(1)).is_err());

    assert!(context69_contracts::strict_metadata_object(&json!({ "a": 1 })).is_ok());
    assert!(context69_contracts::strict_metadata_object(&json!([])).is_err());

    let value = context69_contracts::metadata_object_to_value(&filled);
    assert!(value.is_object());

    let legacy: UpsertLibraryTextRequest = serde_json::from_value(json!({
        "external_id": "doc-1",
        "title": "Doc",
        "content": "Hello"
    }))
    .expect("legacy upsert without metadata");
    assert_eq!(legacy.metadata_json, json!({}));
}

#[test]
fn canonical_ingest_metadata_rejects_non_objects() {
    let valid: CanonicalUploadMetadata = serde_json::from_value(json!({
        "external_id": "doc-1",
        "metadata_json": { "agency": "x" }
    }))
    .expect("canonical metadata with object");
    assert_eq!(valid.metadata_json.get("agency"), Some(&json!("x")));

    let missing: CanonicalUploadMetadata =
        serde_json::from_value(json!({})).expect("canonical metadata defaults");
    assert!(missing.metadata_json.is_empty());

    for bad in [json!([1, 2]), json!("x"), json!(1)] {
        assert!(
            serde_json::from_value::<CanonicalUploadMetadata>(json!({
                "metadata_json": bad
            }))
            .is_err(),
            "canonical metadata must reject {bad}"
        );
        assert!(
            serde_json::from_value::<IngestOptions>(json!({
                "metadata": { "metadata_json": bad }
            }))
            .is_err(),
            "canonical IngestOptions must reject {bad}"
        );
    }

    let legacy_valid = context69_contracts::LibraryFileUploadMetadata {
        external_id: Some("doc-1".to_string()),
        source_uri: None,
        published_at: None,
        metadata_json: json!({ "agency": "x" }),
    };
    let canonical =
        CanonicalUploadMetadata::try_from(legacy_valid).expect("legacy object converts");
    assert_eq!(canonical.metadata_json.get("agency"), Some(&json!("x")));
    let round_trip = context69_contracts::LibraryFileUploadMetadata::from(canonical);
    assert_eq!(round_trip.metadata_json, json!({ "agency": "x" }));

    let legacy_bad = context69_contracts::LibraryFileUploadMetadata {
        external_id: None,
        source_uri: None,
        published_at: None,
        metadata_json: json!([1, 2]),
    };
    assert!(CanonicalUploadMetadata::try_from(legacy_bad).is_err());

    let schema = to_value(schemars::schema_for!(CanonicalUploadMetadata)).expect("schema");
    let text = serde_json::to_string(&schema).expect("schema text");
    assert!(
        text.contains("\"metadata_json\""),
        "schema names metadata_json"
    );
    assert!(
        !text.contains("\"metadata_json\":true"),
        "canonical metadata_json must render as an object, not any"
    );
}

#[test]
fn secret_patch_tri_state_is_explicit() {
    assert_eq!(
        to_value(SecretPatch::Keep).expect("keep"),
        json!({"op": "keep"})
    );
    assert_eq!(
        to_value(SecretPatch::Set("s3cr3t".to_string())).expect("set"),
        json!({"op": "set", "value": "s3cr3t"})
    );
    assert_eq!(
        to_value(SecretPatch::Clear).expect("clear"),
        json!({"op": "clear"})
    );
    assert_eq!(SecretPatch::default(), SecretPatch::Keep);

    assert_eq!(
        SecretPatch::Keep.apply_to(Some("old".to_string())),
        Some("old".to_string())
    );
    assert_eq!(
        SecretPatch::Set("new".to_string()).apply_to(Some("old".to_string())),
        Some("new".to_string())
    );
    assert_eq!(SecretPatch::Clear.apply_to(Some("old".to_string())), None);

    let missing: CanonicalUpdateSearchSettingsRequest = serde_json::from_value(json!({
        "mode": "hybrid",
        "rerank_enabled": false,
        "rerank_base_url": "http://x",
        "rerank_model": "m",
        "candidate_limit": 10,
        "timeout_secs": 5
    }))
    .expect("secret defaults to keep");
    assert_eq!(missing.api_key, SecretPatch::Keep);

    let legacy: context69_contracts::UpdateSearchSettingsRequest = missing.into();
    assert_eq!(legacy.api_key, None);
    assert!(!legacy.clear_api_key);
}
