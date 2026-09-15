//! v0.16 contract parity gate (Redmine 362 Task 1).
//!
//! Reads `docs/contracts/v0.16-inventory.md` (compile-time include, deterministic)
//! and asserts it classifies every OpenAPI operation exactly once with matching
//! method/path, no duplicate operation ids, and that every shared schema name
//! exists in the generated OpenAPI document.

const INVENTORY: &str = include_str!("../docs/contracts/v0.16-inventory.md");

#[derive(Debug, Clone)]
struct InventoryRow {
    operation_id: String,
    method: String,
    path: String,
    request: String,
    response: String,
    errors: String,
    sdk: String,
    mcp: String,
    visibility: String,
}

fn http_section() -> String {
    let start_marker = "## HTTP operations";
    let start = INVENTORY
        .find(start_marker)
        .expect("inventory has ## HTTP operations");
    let rest = &INVENTORY[start..];
    // Next top-level section starts with "\n## ".
    let end = rest["## HTTP operations".len()..]
        .find("\n## ")
        .map(|i| i + "## HTTP operations".len())
        .unwrap_or(rest.len());
    rest[..end].to_string()
}

fn shared_section() -> String {
    let start_marker = "## Shared schemas";
    let start = INVENTORY
        .find(start_marker)
        .expect("inventory has ## Shared schemas");
    let rest = &INVENTORY[start..];
    let end = rest["## Shared schemas".len()..]
        .find("\n## ")
        .map(|i| i + "## Shared schemas".len())
        .unwrap_or(rest.len());
    rest[..end].to_string()
}

fn parse_http_rows() -> Vec<InventoryRow> {
    let section = http_section();
    let mut rows = Vec::new();
    for line in section.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with('|') {
            continue;
        }
        // Leading/trailing pipes give empty ends: raw[0]=="" and raw[last]=="".
        let raw: Vec<&str> = trimmed.split('|').collect();
        if raw.len() < 11 {
            continue;
        }
        let inner: Vec<String> = raw[1..raw.len() - 1]
            .iter()
            .map(|s| s.trim().to_string())
            .collect();
        if inner.len() != 9 {
            panic!(
                "inventory HTTP row must have 9 columns, got {}: {trimmed}",
                inner.len()
            );
        }
        if inner[0] == "operation_id" {
            continue;
        }
        if inner[0].starts_with("---") || inner[1].starts_with("---") {
            continue;
        }
        rows.push(InventoryRow {
            operation_id: inner[0].clone(),
            method: inner[1].clone(),
            path: inner[2].clone(),
            request: inner[3].clone(),
            response: inner[4].clone(),
            errors: inner[5].clone(),
            sdk: inner[6].clone(),
            mcp: inner[7].clone(),
            visibility: inner[8].clone(),
        });
    }
    rows
}

fn parse_shared_schemas() -> Vec<String> {
    let section = shared_section();
    let mut out = Vec::new();
    for line in section.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("- `") {
            continue;
        }
        // Format: - `Name`
        let start = trimmed.find('`').expect("backtick") + 1;
        let end = trimmed[start..].find('`').expect("closing backtick") + start;
        let name = trimmed[start..end].trim().to_string();
        if !name.is_empty() {
            out.push(name);
        }
    }
    out
}

fn unwrap_array(token: &str) -> &str {
    let t = token.trim();
    if let Some(inner) = t.strip_prefix("array<").and_then(|s| s.strip_suffix('>')) {
        inner.trim()
    } else {
        t
    }
}

#[test]
fn inventory_covers_openapi_operations_exactly_once() {
    let rows = parse_http_rows();
    assert!(!rows.is_empty(), "inventory HTTP table must not be empty");

    // No duplicate operation ids in inventory.
    let mut seen = std::collections::HashSet::new();
    for row in &rows {
        assert!(
            !row.operation_id.is_empty(),
            "empty operation_id in row {row:?}"
        );
        assert!(
            seen.insert(row.operation_id.clone()),
            "duplicate operation_id in inventory: {}",
            row.operation_id
        );
        assert!(
            ["GET", "POST", "PUT", "PATCH", "DELETE"].contains(&row.method.as_str()),
            "bad method {} for {}",
            row.method,
            row.operation_id
        );
        assert!(
            ["high-level", "none", "raw"].contains(&row.sdk.as_str()),
            "bad sdk_surface {} for {}",
            row.sdk,
            row.operation_id
        );
        if row.mcp != "none" {
            for part in row.mcp.split(',') {
                let p = part.trim();
                assert!(
                    p.starts_with("tool:") || p.starts_with("resource:"),
                    "bad mcp_surface {p} for {}",
                    row.operation_id
                );
            }
        }
        assert!(
            ["public", "authenticated", "admin"].contains(&row.visibility.as_str()),
            "bad visibility {} for {}",
            row.visibility,
            row.operation_id
        );
    }

    // OpenAPI operations.
    let document = context69::api::openapi_document();
    let value = serde_json::to_value(&document).expect("openapi serializes");
    let paths = value
        .get("paths")
        .and_then(|v| v.as_object())
        .expect("paths exist");
    let mut openapi_ops: Vec<(String, String, String)> = Vec::new();
    let mut openapi_ids = std::collections::HashSet::new();
    for (path, methods) in paths {
        let methods = methods.as_object().expect("path object");
        for method in ["get", "post", "put", "patch", "delete"] {
            if let Some(op) = methods.get(method) {
                let oid = op
                    .get("operationId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                assert!(!oid.is_empty(), "missing operationId for {method} {path}");
                assert!(
                    openapi_ids.insert(oid.clone()),
                    "duplicate operationId in OpenAPI: {oid}"
                );
                openapi_ops.push((oid, method.to_uppercase(), path.clone()));
            }
        }
    }
    assert!(!openapi_ops.is_empty(), "OpenAPI must expose operations");

    // Every OpenAPI operation must be classified.
    let inventory_map: std::collections::HashMap<_, _> =
        rows.iter().map(|r| (r.operation_id.clone(), r)).collect();
    for (oid, method, path) in &openapi_ops {
        let row = inventory_map
            .get(oid)
            .unwrap_or_else(|| panic!("unclassified OpenAPI operation: {oid} {method} {path}"));
        assert_eq!(
            &row.method, method,
            "method mismatch for {oid}: inventory {} vs openapi {method}",
            row.method
        );
        assert_eq!(
            &row.path, path,
            "path mismatch for {oid}: inventory {} vs openapi {path}",
            row.path
        );
    }

    // Every inventory HTTP row must exist in OpenAPI (catches deletions/renames).
    let openapi_map: std::collections::HashMap<_, _> = openapi_ops
        .iter()
        .map(|(oid, method, path)| (oid.clone(), (method.clone(), path.clone())))
        .collect();
    for row in &rows {
        let found = openapi_map.get(&row.operation_id).unwrap_or_else(|| {
            panic!(
                "inventory row not in OpenAPI (stale/deleted): {} {} {}",
                row.operation_id, row.method, row.path
            )
        });
        assert_eq!(
            &row.method, &found.0,
            "method mismatch for inventory row {}",
            row.operation_id
        );
        assert_eq!(
            &row.path, &found.1,
            "path mismatch for inventory row {}",
            row.operation_id
        );
    }

    assert_eq!(
        rows.len(),
        openapi_ops.len(),
        "inventory row count must equal OpenAPI operation count"
    );
}

#[test]
fn inventory_shared_schemas_exist_in_openapi() {
    let shared = parse_shared_schemas();
    assert!(!shared.is_empty(), "Shared schemas section must list names");
    // No duplicates in shared list.
    let mut seen = std::collections::HashSet::new();
    for name in &shared {
        assert!(
            seen.insert(name.clone()),
            "duplicate shared schema in inventory: {name}"
        );
    }

    let document = context69::api::openapi_document();
    let value = serde_json::to_value(&document).expect("openapi serializes");
    let schemas = value
        .pointer("/components/schemas")
        .and_then(|v| v.as_object())
        .expect("schemas exist");
    // v0.18 breaking Task B2: the flattened legacy upload schemas are
    // intentionally gone from OpenAPI while the frozen v0.16 inventory
    // still lists them.
    const B2_REMOVED_SCHEMAS: &[&str] = &["LibraryFileUploadMetadata", "LibraryFileIngestOptions"];
    for name in &shared {
        if B2_REMOVED_SCHEMAS.contains(&name.as_str()) {
            assert!(
                !schemas.contains_key(name),
                "B2-removed schema must stay gone from OpenAPI: {name}"
            );
            continue;
        }
        assert!(
            schemas.contains_key(name),
            "inventory shared schema missing in OpenAPI: {name}"
        );
    }

    // Every schema token referenced by HTTP rows must be listed as shared
    // (or be a placeholder).
    let shared_set: std::collections::HashSet<_> = shared.iter().collect();
    for row in parse_http_rows() {
        for token in [row.request.clone(), row.response.clone()] {
            for part in token.split('+') {
                let base = unwrap_array(part);
                if base == "-" || base == "multipart-form" || base.is_empty() {
                    continue;
                }
                assert!(
                    shared_set.contains(&base.to_string()),
                    "row {} references schema {base} not listed in Shared schemas",
                    row.operation_id
                );
                assert!(
                    schemas.contains_key(base),
                    "row {} references schema {base} missing in OpenAPI",
                    row.operation_id
                );
            }
        }
        if row.errors != "-" {
            for entry in row.errors.split(';') {
                let e = entry.trim();
                if e.is_empty() {
                    continue;
                }
                if let Some((_, schema)) = e.split_once(':') {
                    let base = unwrap_array(schema);
                    if base == "-" || base.is_empty() {
                        continue;
                    }
                    assert!(
                        shared_set.contains(&base.to_string()),
                        "row {} error references schema {base} not in Shared schemas",
                        row.operation_id
                    );
                    assert!(
                        schemas.contains_key(base),
                        "row {} error schema {base} missing in OpenAPI",
                        row.operation_id
                    );
                }
            }
        }
    }

    // Core frozen schemas must be present.
    for required in [
        "HealthResponse",
        "ApiErrorResponse",
        "SearchRequest",
        "SearchResponse",
        "TaskPageResponse",
        "TaskResponse",
        "SourceStatus",
        "DocumentResponse",
    ] {
        assert!(
            shared_set.contains(&required.to_string()),
            "Shared schemas must contain {required}"
        );
    }
}

#[test]
fn inventory_covers_sdk_facade_and_mcp_surfaces() {
    // SDK facade methods on Context69Client (issue 391 Task 4: retention/purge
    // facade removed, recovery ops preserved).
    for method in [
        "ensure_scope",
        "text_batch",
        "url_batch",
        "file_batch",
        "delete_batch",
        "release_file_source",
        "submit_task",
        "task",
        "tasks",
        "task_items",
        "wait",
        "wait_with_options",
        "retry_task",
        "rerun_task",
        "cancel_task",
        "trash_task",
        "restore_task",
        "delete_task",
        "cancel_active_tasks",
        "search_compact",
        "get_document",
        "get_document_by_key",
        "get_documents",
        "list_extraction_templates",
        "upsert_extraction_template",
        "document_extractions",
        "rebuild_document_extractions",
        "me",
        "healthz",
    ] {
        assert!(
            INVENTORY.contains(&format!("`{method}`")),
            "inventory SDK section must mention `{method}`"
        );
    }
    // MCP tools.
    for tool in [
        "search_documents",
        "get_document",
        "query_documents",
        "get_document_by_external_id",
        "get_documents",
        "list_sources",
    ] {
        assert!(
            INVENTORY.contains(tool),
            "inventory MCP section must mention tool {tool}"
        );
    }
    // MCP resources.
    for resource in [
        "context69://sources/{source_key}",
        "context69://documents/{document_id}",
    ] {
        assert!(
            INVENTORY.contains(resource),
            "inventory MCP section must mention resource {resource}"
        );
    }
}
