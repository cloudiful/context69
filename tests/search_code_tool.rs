//! `search_code` MCP contract tests (issue #681 work unit 3C3).
//!
//! Pure contract checks with no database and no provider: the tool input
//! deserializes with truthful schema bounds, invalid terms/filters/limits are
//! rejected, stored lexical hits project into bounded hits without losing
//! provenance, and the response reports truncation instead of a cursor.

use chrono::Utc;
use context69::contracts::sources::{GitCodeLexicalHit, GitCodeMatchKind};
use context69::contracts::{
    MCP_CODE_LIMIT_DEFAULT, MCP_CODE_LIMIT_MAX, MCP_CODE_QUERY_MAX_CHARS, MCP_CODE_TEXT_MAX_CHARS,
    McpCodeCoverage, McpCodeGeneration, McpCodeHit, McpCodeSearchRequest, McpCodeSearchResponse,
    Visibility,
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

fn lexical_hit(text: &str) -> GitCodeLexicalHit {
    GitCodeLexicalHit {
        repository_key: Uuid::new_v4(),
        generation_key: Uuid::new_v4(),
        generation_number: 7,
        ref_name: "refs/heads/main".to_string(),
        commit_sha: "a".repeat(40),
        visibility: Visibility::Private,
        file_key: Uuid::new_v4(),
        path: "src/mcp/tools/code.rs".to_string(),
        language: "rust".to_string(),
        chunk_key: Uuid::new_v4(),
        chunk_index: 2,
        start_line: 10,
        end_line: 20,
        text: text.to_string(),
        score: 0.94,
        matched: GitCodeMatchKind::ChunkPhrase,
    }
}

fn generation() -> McpCodeGeneration {
    McpCodeGeneration::new(
        Uuid::new_v4(),
        Uuid::new_v4(),
        7,
        "refs/heads/main",
        &"b".repeat(40),
        Some(Utc::now()),
        McpCodeCoverage {
            file_count: 12,
            excluded_file_count: 3,
            total_bytes: 4_096,
        },
    )
}

#[test]
fn request_schema_declares_truthful_bounds() {
    let schema = schema_of::<McpCodeSearchRequest>();
    assert_eq!(
        keyword(&schema, "query", "maxLength"),
        json!(MCP_CODE_QUERY_MAX_CHARS)
    );
    assert_eq!(keyword(&schema, "query", "minLength"), json!(1));
    assert_eq!(keyword(&schema, "group_path", "maxLength"), json!(1024));
    assert_eq!(keyword(&schema, "path_prefix", "maxLength"), json!(512));
    assert_eq!(keyword(&schema, "language", "maxLength"), json!(32));
    assert_eq!(keyword(&schema, "limit", "minimum"), json!(1));
    assert_eq!(
        keyword(&schema, "limit", "maximum"),
        json!(MCP_CODE_LIMIT_MAX)
    );
    let required = schema
        .get("required")
        .and_then(Value::as_array)
        .expect("request schema declares required fields");
    for field in ["group_path", "repository_key", "query"] {
        assert!(
            required.iter().any(|value| value == field),
            "request schema must require {field}: {schema}"
        );
    }
}

#[test]
fn hit_and_response_schemas_declare_truthful_bounds() {
    let hit = schema_of::<McpCodeHit>();
    assert_eq!(
        keyword(&hit, "text", "maxLength"),
        json!(MCP_CODE_TEXT_MAX_CHARS)
    );
    assert_eq!(keyword(&hit, "path", "maxLength"), json!(512));
    assert_eq!(keyword(&hit, "language", "maxLength"), json!(32));
    assert_eq!(keyword(&hit, "ref_name", "maxLength"), json!(256));
    assert_eq!(keyword(&hit, "commit_sha", "maxLength"), json!(64));

    let response = schema_of::<McpCodeSearchResponse>();
    assert_eq!(
        keyword(&response, "hits", "maxItems"),
        json!(MCP_CODE_LIMIT_MAX)
    );
    assert_eq!(keyword(&response, "truncated", "type"), json!("boolean"));
    assert!(
        prop(&response, "generation").get("$ref").is_some(),
        "response carries bounded generation provenance: {response}"
    );
}

#[test]
fn request_deserializes_with_defaults_and_rejects_http_fields() {
    let repository_key = Uuid::new_v4();
    let documented: McpCodeSearchRequest = serde_json::from_value(json!({
        "group_path": "research/context69",
        "repository_key": repository_key,
        "query": "search_code",
        "path_prefix": "src/mcp/",
        "language": "rust",
        "limit": 10,
    }))
    .expect("documented search_code payload deserializes");
    assert!(documented.validate().is_ok());
    assert_eq!(documented.repository_key, repository_key);

    let minimal: McpCodeSearchRequest = serde_json::from_value(json!({
        "group_path": "research/context69",
        "repository_key": repository_key,
        "query": "search_code",
    }))
    .expect("minimal payload deserializes");
    assert_eq!(minimal.limit, MCP_CODE_LIMIT_DEFAULT);
    assert!(minimal.path_prefix.is_none());
    assert!(minimal.validate().is_ok());

    for extra in [json!({"cursor": "next"}), json!({"page": 2})] {
        let mut payload = json!({
            "group_path": "research/context69",
            "repository_key": repository_key,
            "query": "search_code",
        });
        for (key, value) in extra.as_object().expect("object") {
            payload[key] = value.clone();
        }
        assert!(
            serde_json::from_value::<McpCodeSearchRequest>(payload).is_err(),
            "HTTP-only fields must be rejected, not silently ignored"
        );
    }
}

#[test]
fn request_validation_rejects_blank_oversized_and_invalid_values() {
    let repository_key = Uuid::new_v4();
    let base = json!({
        "group_path": "research/context69",
        "repository_key": repository_key,
        "query": "search_code",
    });

    let cases: Vec<(Value, &str)> = vec![
        (json!({"query": "   "}), "blank query"),
        (
            json!({"query": "x".repeat(MCP_CODE_QUERY_MAX_CHARS + 1)}),
            "oversized query",
        ),
        (json!({"limit": 0}), "limit below minimum"),
        (
            json!({"limit": MCP_CODE_LIMIT_MAX + 1}),
            "limit above maximum",
        ),
        (json!({"path_prefix": "  "}), "blank path_prefix"),
        (
            json!({"path_prefix": "x".repeat(513)}),
            "oversized path_prefix",
        ),
        (json!({"path_prefix": "/absolute"}), "absolute path_prefix"),
        (
            json!({"path_prefix": "src/../secret"}),
            "traversing path_prefix",
        ),
        (
            json!({"path_prefix": "src/\u{1}"}),
            "control character path_prefix",
        ),
        (json!({"language": "Rust"}), "uppercase language"),
        (json!({"language": "rust lang"}), "space in language"),
        (json!({"language": "x".repeat(33)}), "oversized language"),
    ];

    for (patch, label) in cases {
        let mut payload = base.clone();
        for (key, value) in patch.as_object().expect("object") {
            payload[key] = value.clone();
        }
        let request: McpCodeSearchRequest =
            serde_json::from_value(payload.clone()).expect("payload shape deserializes");
        assert!(
            request.validate().is_err(),
            "must reject {label}: {payload}"
        );
    }

    let valid: McpCodeSearchRequest = serde_json::from_value(json!({
        "group_path": "research/context69",
        "repository_key": repository_key,
        "query": "search_code",
        "path_prefix": "src/mcp",
        "language": "rust",
        "limit": MCP_CODE_LIMIT_MAX,
    }))
    .expect("valid payload deserializes");
    assert!(valid.validate().is_ok());
}

#[test]
fn projection_truncates_text_and_preserves_provenance() {
    let long_text = "x".repeat(MCP_CODE_TEXT_MAX_CHARS + 250);
    let mut raw = lexical_hit(&long_text);
    raw.path = "p".repeat(700);
    raw.language = "l".repeat(80);
    raw.ref_name = "r".repeat(300);
    raw.commit_sha = "c".repeat(90);

    let hit = McpCodeHit::from_lexical_hit(&raw);
    assert_eq!(hit.repository_key, raw.repository_key);
    assert_eq!(hit.generation_key, raw.generation_key);
    assert_eq!(hit.generation_number, raw.generation_number);
    assert_eq!(hit.file_key, raw.file_key);
    assert_eq!(hit.chunk_key, raw.chunk_key);
    assert_eq!(hit.chunk_index, raw.chunk_index);
    assert_eq!(hit.start_line, raw.start_line);
    assert_eq!(hit.end_line, raw.end_line);
    assert_eq!(hit.score, raw.score);
    assert_eq!(hit.matched, raw.matched);

    assert_eq!(hit.text.chars().count(), MCP_CODE_TEXT_MAX_CHARS);
    assert!(
        long_text.starts_with(&hit.text),
        "text stays a verbatim prefix"
    );
    assert_eq!(hit.path.chars().count(), 512);
    assert_eq!(hit.language.chars().count(), 32);
    assert_eq!(hit.ref_name.chars().count(), 256);
    assert_eq!(hit.commit_sha.chars().count(), 64);

    // Verbatim content is not rewritten: punctuation and identifiers survive
    // whenever they fit under the cap.
    let short = lexical_hit("let x = SafeRef::parse(&raw)?; // 100%");
    let projected = McpCodeHit::from_lexical_hit(&short);
    assert_eq!(projected.text, short.text);
    assert_eq!(projected.path, short.path);
}

#[test]
fn response_reports_truncation_without_a_cursor() {
    let hits = vec![
        McpCodeHit::from_lexical_hit(&lexical_hit("one")),
        McpCodeHit::from_lexical_hit(&lexical_hit("two")),
        McpCodeHit::from_lexical_hit(&lexical_hit("three")),
    ];

    let truncated = McpCodeSearchResponse::new(generation(), hits.clone(), 2);
    assert!(truncated.truncated);
    assert_eq!(truncated.hits.len(), 2);
    assert_eq!(truncated.generation.coverage.file_count, 12);
    assert_eq!(truncated.generation.coverage.excluded_file_count, 3);
    assert_eq!(truncated.generation.coverage.total_bytes, 4_096);
    assert_eq!(truncated.generation.generation_number, 7);
    assert_eq!(truncated.generation.ref_name, "refs/heads/main");
    assert_eq!(truncated.generation.commit_sha, "b".repeat(40));

    let exact = McpCodeSearchResponse::new(generation(), hits, 3);
    assert!(!exact.truncated);
    assert_eq!(exact.hits.len(), 3);

    let serialized = serde_json::to_value(&truncated).expect("response serializes");
    assert_eq!(serialized["truncated"], json!(true));
    assert!(
        serialized.get("next_cursor").is_none(),
        "search_code has no cursor: {serialized}"
    );
    assert!(
        serialized.get("has_more").is_none(),
        "search_code reports truncation instead of has_more: {serialized}"
    );
}

#[test]
fn generation_projection_truncates_provenance() {
    let meta = McpCodeGeneration::new(
        Uuid::new_v4(),
        Uuid::new_v4(),
        3,
        &"r".repeat(300),
        &"c".repeat(90),
        None,
        McpCodeCoverage {
            file_count: 0,
            excluded_file_count: 0,
            total_bytes: 0,
        },
    );
    assert_eq!(meta.ref_name.chars().count(), 256);
    assert_eq!(meta.commit_sha.chars().count(), 64);
    assert!(meta.completed_at.is_none());
}
