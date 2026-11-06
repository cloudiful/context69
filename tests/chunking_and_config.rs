use context69::{
    chunking::chunking_token::{TokenizerKind, token_count},
    chunking::{ChunkBudget, ChunkingConfig, chunk_document, chunk_document_with_budget},
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

#[test]
fn token_budget_chunking_is_reachable_through_the_public_api() {
    let document = NormalizedDocument {
        external_id: "doc-1".to_string(),
        title: "Doc".to_string(),
        summary: Some("summary".to_string()),
        body_text: format!(
            "{}\n\n{}",
            "The quick brown fox jumps over the lazy dog. ".repeat(10),
            "A second paragraph keeps the token chunker honest. ".repeat(10)
        ),
        source_uri: "https://example.com/doc-1".to_string(),
        published_at: None,
        updated_at: Utc::now(),
        metadata_json: json!({}),
        record_hash: "hash-1".to_string(),
    };

    let budget = ChunkBudget::tokens(TokenizerKind::Cl100kBase, 48, 12);
    let chunks = chunk_document_with_budget(42, "source-a", &document, &budget)
        .expect("the token budget should be valid");

    assert!(
        chunks.len() >= 2,
        "the token budget must split the document"
    );
    for chunk in &chunks {
        assert_eq!(chunk.document_id, 42);
        assert!(
            token_count(&chunk.text, TokenizerKind::Cl100kBase) <= 48,
            "chunk {} exceeds the token budget",
            chunk.chunk_index
        );
    }
}

#[test]
fn a_character_budget_matches_the_legacy_chunking_through_the_public_api() {
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
    let config = ChunkingConfig {
        max_chars: 900,
        overlap_chars: 100,
    };

    let legacy = chunk_document(42, "source-a", &document, &config);
    let budgeted =
        chunk_document_with_budget(42, "source-a", &document, &ChunkBudget::from(&config))
            .expect("the character budget is always valid");

    assert_eq!(
        budgeted
            .iter()
            .map(|chunk| (chunk.id, chunk.text.as_str(), chunk.chunk_index))
            .collect::<Vec<_>>(),
        legacy
            .iter()
            .map(|chunk| (chunk.id, chunk.text.as_str(), chunk.chunk_index))
            .collect::<Vec<_>>()
    );
}
