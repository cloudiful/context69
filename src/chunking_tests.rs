use chrono::Utc;
use serde_json::json;

use super::{ChunkingConfig, chunk_document, chunk_document_iter};
use crate::domain::NormalizedDocument;

fn document(body_text: &str) -> NormalizedDocument {
    NormalizedDocument {
        external_id: "doc-1".to_string(),
        title: "title".to_string(),
        summary: None,
        body_text: body_text.to_string(),
        source_uri: "https://example.com".to_string(),
        published_at: None,
        updated_at: Utc::now(),
        metadata_json: json!({}),
        record_hash: "hash".to_string(),
    }
}

#[test]
fn lazy_iterator_matches_collecting_api() {
    let document = document(&format!("{}\n\n{}", "a".repeat(900), "b".repeat(900)));
    let config = ChunkingConfig {
        max_chars: 1000,
        overlap_chars: 100,
    };

    let collected = chunk_document(1, "gov-info", &document, &config);
    let streamed = chunk_document_iter(1, "gov-info", &document, &config).collect::<Vec<_>>();

    assert_eq!(
        streamed
            .iter()
            .map(|chunk| chunk.text.as_str())
            .collect::<Vec<_>>(),
        collected
            .iter()
            .map(|chunk| chunk.text.as_str())
            .collect::<Vec<_>>()
    );
    assert!(collected.len() >= 2);
    assert!(collected[0].text.len() >= 900);
    assert!(
        collected
            .iter()
            .any(|chunk| chunk.text.contains(&"b".repeat(300)))
    );
}

#[test]
fn long_paragraph_keeps_all_text_with_overlap() {
    let document = document("abcdefghijklmnop");
    let chunks = chunk_document(
        1,
        "file_library",
        &document,
        &ChunkingConfig {
            max_chars: 5,
            overlap_chars: 2,
        },
    );

    assert_eq!(
        chunks
            .iter()
            .map(|chunk| chunk.text.as_str())
            .collect::<Vec<_>>(),
        ["abcde", "defgh", "ghijk", "jklmn", "mnop"]
    );
}

#[test]
fn paragraph_boundary_retains_separator_and_tail() {
    let document = document("abcd\n\nefghij");
    let chunks = chunk_document(
        1,
        "file_library",
        &document,
        &ChunkingConfig {
            max_chars: 10,
            overlap_chars: 2,
        },
    );

    assert_eq!(
        chunks
            .iter()
            .map(|chunk| chunk.text.as_str())
            .collect::<Vec<_>>(),
        ["abcd", "cd\n\nefghij"]
    );
}

#[test]
fn zero_overlap_does_not_repeat_full_chunks() {
    let document = document("abcdefghij");
    let chunks = chunk_document(
        1,
        "file_library",
        &document,
        &ChunkingConfig {
            max_chars: 4,
            overlap_chars: 0,
        },
    );

    assert_eq!(
        chunks
            .iter()
            .map(|chunk| chunk.text.as_str())
            .collect::<Vec<_>>(),
        ["abcd", "efgh", "ij"]
    );
}
