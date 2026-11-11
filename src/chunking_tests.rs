use chrono::Utc;
use serde_json::json;

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

#[test]
fn the_two_token_versions_keep_their_own_serialized_identity() {
    use super::ChunkBudgetMode;
    use crate::chunking::chunking_token::TokenizerKind;

    let legacy = ChunkBudget::tokens(TokenizerKind::O200kBase, 1024, 128);
    assert_eq!(legacy.identity(), "tokens:o200k_base:1024:128");
    let serialized = serde_json::to_value(&legacy).expect("the legacy budget serializes");
    assert_eq!(serialized["mode"], json!("tokens"));
    let parsed = super::parse_chunk_budget_value(&serialized).expect("the legacy budget parses");
    assert_eq!(parsed.mode, ChunkBudgetMode::Tokens);
    assert_eq!(parsed.identity(), legacy.identity());

    let bounded = ChunkBudget::tokens_v2(TokenizerKind::O200kBase, 1024, 128);
    assert_eq!(bounded.identity(), "tokens_v2:o200k_base:1024:128");
    let serialized = serde_json::to_value(&bounded).expect("the bounded budget serializes");
    assert_eq!(serialized["mode"], json!("tokens_v2"));
    let parsed = super::parse_chunk_budget_value(&serialized).expect("the bounded budget parses");
    assert_eq!(
        parsed.mode,
        ChunkBudgetMode::TokensV2,
        "a parsed budget keeps the version it was stored with"
    );
    assert_eq!(parsed.identity(), bounded.identity());
}

#[test]
fn an_unknown_token_budget_version_fails_closed() {
    use crate::chunking::chunking_token::TokenizerKind;

    let mut budget = serde_json::to_value(ChunkBudget::tokens(TokenizerKind::Cl100kBase, 512, 64))
        .expect("the budget serializes");
    budget["mode"] = json!("tokens_v3");

    let error = super::parse_chunk_budget_value(&budget)
        .expect_err("an unknown stored version must not be read as a known one");
    assert!(
        matches!(error, super::ChunkingError::InvalidSnapshot { .. }),
        "{error}"
    );
}

#[test]
fn fresh_budgets_select_the_bounded_token_version() {
    use super::ChunkBudgetMode;
    use crate::chunking::chunking_token::TokenizerKind;
    use std::path::PathBuf;

    assert_eq!(
        ChunkBudgetMode::Tokens.fresh(),
        ChunkBudgetMode::TokensV2,
        "a fresh token budget uses the bounded cut"
    );
    assert_eq!(ChunkBudgetMode::Chars.fresh(), ChunkBudgetMode::Chars);
    assert_eq!(ChunkBudgetMode::TokensV2.fresh(), ChunkBudgetMode::TokensV2);

    let config = crate::config::ChunkingBudgetConfig {
        max_chars: 1200,
        overlap_chars: 200,
        unit: ChunkBudgetMode::Tokens,
        tokenizer: TokenizerKind::O200kBase,
        tokenizer_sha256: None,
        tokenizer_artifact_dir: PathBuf::from("/var/lib/context69/tokenizers"),
        max_tokens: 512,
        overlap_tokens: 64,
    };
    let budget = config.chunk_budget();
    assert_eq!(budget.mode, ChunkBudgetMode::TokensV2);
    assert_eq!(budget.identity(), "tokens_v2:o200k_base:512:64");

    let characters = crate::config::ChunkingBudgetConfig {
        unit: ChunkBudgetMode::Chars,
        ..config
    };
    assert_eq!(characters.chunk_budget().identity(), "chars:1200:200");
}
