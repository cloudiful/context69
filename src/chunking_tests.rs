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
    let streamed = chunk_document_iter(1, "gov-info", &document, &config, None).collect::<Vec<_>>();

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

#[test]
fn absent_generation_keeps_preexisting_chunk_ids() {
    let body = format!("{}\n\n{}", "a".repeat(900), "b".repeat(900));
    let document = document(&body);
    let chunks = chunk_document_iter(1, "gov-info", &document, &ChunkingConfig::default(), None);

    let ids = chunks
        .into_iter()
        .map(|chunk| chunk.id.to_string())
        .collect::<Vec<_>>();

    assert_eq!(
        ids,
        [
            "0c8ea280-9525-596d-8711-a0038e22eef4",
            "b0bafd67-897c-5b03-b5dc-eb954b3e0c0f",
        ]
    );
}

#[test]
fn generation_produces_stable_distinct_ids() {
    let body = format!("{}\n\n{}", "a".repeat(900), "b".repeat(900));
    let document = document(&body);
    let config = ChunkingConfig {
        max_chars: 1000,
        overlap_chars: 100,
    };

    let none_ids = chunk_document(1, "gov-info", &document, &config)
        .into_iter()
        .map(|chunk| chunk.id)
        .collect::<Vec<_>>();

    let run = |generation: Option<i64>| {
        chunk_document_iter(1, "gov-info", &document, &config, generation)
            .map(|chunk| chunk.id)
            .collect::<Vec<_>>()
    };

    let first = run(Some(7));
    let second = run(Some(7));
    let other = run(Some(8));

    assert_eq!(
        first, second,
        "the same generation must be deterministic across repeats"
    );
    for (index, (ungenerated, generated)) in none_ids.iter().zip(&first).enumerate() {
        assert_ne!(
            ungenerated, generated,
            "a supplied generation must differ from the absent-generation id (index {index})"
        );
        assert_ne!(
            first[index], other[index],
            "different generations must produce distinct ids (index {index})"
        );
    }
    assert!(!first.is_empty());
}

#[test]
fn generation_keys_are_unambiguous() {
    use uuid::Uuid;

    let chunk_uuid = |source_key: &str,
                      external_id: &str,
                      record_hash: &str,
                      chunk_index: i32,
                      generation: Option<i64>| {
        super::chunk_uuid(
            source_key,
            external_id,
            record_hash,
            chunk_index,
            generation,
        )
    };

    let plain = chunk_uuid("gov-info", "doc-1", "hash", 0, None);
    let generated = chunk_uuid("gov-info", "doc-1", "hash", 0, Some(7));

    let preexisting_name = "gov-info:doc-1:hash:0";
    assert_eq!(
        plain,
        Uuid::new_v5(&Uuid::NAMESPACE_URL, preexisting_name.as_bytes()),
        "the absent-generation key must stay the pre-change key"
    );
    assert_eq!(
        generated,
        Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            format!("{preexisting_name}:7").as_bytes()
        ),
        "the generation key must be the pre-change key plus an unambiguous suffix"
    );
    assert_ne!(plain, generated);
    assert_ne!(
        generated,
        chunk_uuid("gov-info", "doc-1", "hash", 7, None),
        "a generated id must not equal any pre-existing (source_key, external_id, record_hash, index) id"
    );
    assert_ne!(
        chunk_uuid("gov-info", "doc-1", "hash", 0, Some(17)),
        chunk_uuid("gov-info", "doc-1", "hash", 1, Some(7)),
        "generation and chunk index must not collide inside the key"
    );

    for generation in [0, 1, 7, 17, 20260101] {
        let encoded = format!("gov-info:doc-1:hash:0:{generation}");
        let (text, suffix) = encoded.rsplit_once(':').expect("one suffix segment");
        assert_eq!(text, preexisting_name);
        assert_eq!(
            suffix
                .parse::<i64>()
                .expect("the suffix must decode to the generation"),
            generation,
            "the generation must survive a lossless encode/decode round trip"
        );
    }
}
