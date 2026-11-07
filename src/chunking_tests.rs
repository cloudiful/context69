use chrono::Utc;
use serde_json::json;

use super::chunking_token::TokenizerKind;
use super::{ChunkBudget, ChunkingConfig, chunk_document, chunk_document_iter};
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

#[test]
fn a_character_budget_keeps_the_legacy_chunks_and_ids() {
    let body = format!("{}\n\n{}", "a".repeat(900), "b".repeat(900));
    let document = document(&body);
    let config = ChunkingConfig {
        max_chars: 1000,
        overlap_chars: 100,
    };

    let legacy = chunk_document(1, "gov-info", &document, &config);
    let budgeted =
        super::chunk_document_with_budget(1, "gov-info", &document, &ChunkBudget::from(&config))
            .expect("the character budget is always valid");

    assert_eq!(
        budgeted
            .iter()
            .map(|chunk| (chunk.id, chunk.text.as_str()))
            .collect::<Vec<_>>(),
        legacy
            .iter()
            .map(|chunk| (chunk.id, chunk.text.as_str()))
            .collect::<Vec<_>>(),
        "the character budget must produce the legacy chunks and ids"
    );
}

#[test]
fn a_malformed_character_snapshot_is_refused_without_changing_the_legacy_api() {
    for (max_chars, overlap_chars) in [(0, 0), (10, 10)] {
        let budget = ChunkBudget::chars(ChunkingConfig {
            max_chars,
            overlap_chars,
        });
        let payload = json!({
            super::CHUNK_BUDGET_PAYLOAD_KEY: serde_json::to_value(&budget)
                .expect("the budget serializes")
        });
        let error = super::parse_chunk_budget(&payload)
            .expect_err("a present malformed character snapshot must fail closed");
        assert!(
            matches!(error, super::ChunkingError::InvalidSnapshot { .. }),
            "{max_chars}/{overlap_chars}: {error}"
        );
    }

    // The legacy character API keeps its historical coercion of the same
    // values; only the persisted snapshot boundary refuses them.
    let document = document("abcdefghij");
    let chunks = chunk_document(
        1,
        "gov-info",
        &document,
        &ChunkingConfig {
            max_chars: 0,
            overlap_chars: 10,
        },
    );
    assert_eq!(
        chunks.len(),
        10,
        "the legacy iterator still yields one char"
    );
}

#[test]
fn a_character_budget_reports_the_same_chunks_as_the_legacy_api() {
    let body = format!("{}\n\n{}", "a".repeat(900), "b".repeat(900));
    let document = document(&body);
    let config = ChunkingConfig {
        max_chars: 1000,
        overlap_chars: 100,
    };

    let chunks =
        super::chunk_document_with_budget(1, "gov-info", &document, &ChunkBudget::from(&config))
            .expect("the character budget is always valid");

    assert_eq!(
        chunks
            .iter()
            .map(|chunk| chunk.text.as_str())
            .collect::<Vec<_>>(),
        chunk_document(1, "gov-info", &document, &config)
            .iter()
            .map(|chunk| chunk.text.as_str())
            .collect::<Vec<_>>()
    );
}

/// The character path enforces the same complete embedding envelope at its
/// emission point and never drops the tail of an oversized paragraph.
#[test]
fn the_input_envelope_splits_an_oversized_char_paragraph_and_preserves_bytes() {
    let prefix = "system:";
    let body: String = (0..400).map(|index| format!("word{index} ")).collect();
    let sample = &body[..body.len().min(160)];
    let mut policy = context69_contracts::settings::EmbeddingInputPolicy::default();
    policy.document_server_prefix = prefix.to_string();
    policy.max_input_tokens = Some(super::chunking_token::token_count(
        &format!("{prefix}{sample}"),
        super::chunking_token::TokenizerKind::Cl100kBase,
    ));
    let mut budget = ChunkBudget::chars(ChunkingConfig {
        max_chars: 400,
        overlap_chars: 50,
    });
    budget.input_policy = Some(policy.clone());
    let runtime = super::ChunkingRuntime::new(budget.clone());
    let resolved = runtime
        .resolve(&budget)
        .expect("resolve the pinned envelope");
    let chunks = resolved
        .chunk_document(1, "file_library", &document(&body))
        .expect("the envelope chunks the source");
    assert!(chunks.len() > 1, "the envelope must split the paragraph");
    let envelope = resolved.input_policy().expect("the policy is pinned");
    let cap = policy.max_input_tokens.expect("the cap is set");
    let mut source = String::new();
    for (index, chunk) in chunks.iter().enumerate() {
        assert!(
            envelope.count_input(prefix, &chunk.text) <= cap,
            "every emitted chunk fits the complete envelope"
        );
        let overlap = if index == 0 {
            String::new()
        } else {
            let previous = &chunks[index - 1].text;
            let chars: Vec<char> = previous.chars().collect();
            chars[chars.len().saturating_sub(50)..].iter().collect()
        };
        assert!(
            chunk.text.starts_with(&overlap),
            "chunk {index} repeats the configured overlap"
        );
        source.push_str(&chunk.text[overlap.len()..]);
    }
    assert_eq!(source, body.trim());
}

/// Legacy budgets serialize and identify exactly as before, while a nondefault
/// policy adds a collision-safe component.
#[test]
fn the_input_policy_identity_is_versioned_and_only_added_when_nondefault() {
    let legacy = ChunkBudget::tokens(TokenizerKind::O200kBase, 512, 64);
    assert_eq!(legacy.identity(), "tokens:o200k_base:512:64");
    let value = super::chunk_budget_to_value(&legacy);
    assert!(value.get("input_policy").is_none(), "{value}");

    let mut first = legacy.clone();
    let mut policy = context69_contracts::settings::EmbeddingInputPolicy::default();
    policy.max_input_tokens = Some(1024);
    policy.document_server_prefix = "a".repeat(2);
    policy.query_server_prefix = "b".to_string();
    first.input_policy = Some(policy.clone());
    let mut second = first.clone();
    let mut other = policy.clone();
    other.document_server_prefix = "a".to_string();
    other.query_server_prefix = "ab".to_string();
    second.input_policy = Some(other);
    let first_identity = first.identity();
    let second_identity = second.identity();
    assert_ne!(
        first_identity, second_identity,
        "length-prefixed fields do not collide"
    );
    assert_eq!(first_identity, first.identity());
    assert!(first_identity.starts_with("tokens:o200k_base:512:64:input:"));

    first.input_policy = Some(context69_contracts::settings::EmbeddingInputPolicy::default());
    assert_eq!(
        first.identity(),
        "tokens:o200k_base:512:64",
        "an explicit default policy keeps the legacy identity"
    );
}
