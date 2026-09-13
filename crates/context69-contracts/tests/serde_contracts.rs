use context69_contracts::{
    CreateTextRequest, ImportLibraryFileFromUrlRequest, LibraryTextContentFormat, Pagination,
    PersonalAccessTokenScope, UpsertLibraryTextRequest,
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
    let exact_value = to_value(&exact).expect("serialize exact");
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
    let window_value = to_value(&window).expect("serialize window");
    assert_eq!(window_value.get("has_more"), Some(&json!(true)));
    assert_eq!(window_value.get("total_is_exact"), Some(&json!(false)));

    // Unknown probe state round-trips without claiming end-of-results.
    let capped = Pagination::try_new_search_window(5, 8, 2_000, None).expect("capped window");
    let capped_value = to_value(&capped).expect("serialize capped");
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
