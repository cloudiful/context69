use context69::{
    chunking::{ChunkingConfig, chunk_document},
    models::NormalizedDocument,
};

use chrono::Utc;
use rmcp::schemars::schema_for;
use serde_json::json;

#[test]
fn chunking_keeps_stable_chunk_indexes() {
    let document = NormalizedDocument {
        external_id: "doc-1".to_string(),
        title: "Doc".to_string(),
        summary: Some("summary".to_string()),
        body_text: format!("{}\n\n{}", "a".repeat(700), "b".repeat(700)),
        source_uri: "https://example.com/doc-1".to_string(),
        published_at: None,
        updated_at: Utc::now(),
        metadata_json: json!({}),
        record_hash: "hash-1".to_string(),
    };

    let chunks = chunk_document(
        42,
        "source-a",
        &document,
        &ChunkingConfig {
            max_chars: 900,
            overlap_chars: 100,
        },
    );

    assert!(chunks.len() >= 2);
    assert_eq!(chunks[0].chunk_index, 0);
    assert_eq!(chunks[1].chunk_index, 1);
    assert_eq!(chunks[0].document_id, 42);
}

#[test]
fn search_request_schema_includes_rest_filters_for_mcp_reuse() {
    let schema = schema_for!(context69::models::SearchRequest);
    let schema_json = serde_json::to_value(&schema).expect("schema should serialize");
    let properties = schema_json
        .get("properties")
        .and_then(serde_json::Value::as_object)
        .expect("search request schema should expose properties");

    assert!(properties.contains_key("query"));
    assert!(properties.contains_key("limit"));
    assert!(properties.contains_key("source_key"));
    assert!(properties.contains_key("published_after"));
    assert!(properties.contains_key("published_before"));
}
