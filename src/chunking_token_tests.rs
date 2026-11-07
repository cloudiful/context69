use chrono::Utc;
use serde_json::json;

use super::chunking_token::{TokenizerKind, token_count, token_suffix};
use super::{ChunkBudget, ChunkingError};
use crate::domain::{DocumentChunk, NormalizedDocument};

const TOKENIZERS: [TokenizerKind; 2] = [TokenizerKind::Cl100kBase, TokenizerKind::O200kBase];

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

fn budget(tokenizer: TokenizerKind, max_tokens: usize, overlap_tokens: usize) -> ChunkBudget {
    ChunkBudget::tokens(tokenizer, max_tokens, overlap_tokens)
}

fn normalized_source(body_text: &str) -> String {
    body_text
        .split("\n\n")
        .map(str::trim)
        .filter(|paragraph| !paragraph.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Split the chunks into their novel content by removing the overlap prefix
/// that the engine repeats from the previous chunk.
fn contents(
    chunks: &[DocumentChunk],
    tokenizer: TokenizerKind,
    overlap_tokens: usize,
) -> Vec<String> {
    let mut contents: Vec<String> = Vec::with_capacity(chunks.len());
    for (index, chunk) in chunks.iter().enumerate() {
        let overlap = match contents.last() {
            Some(previous) => token_suffix(previous, tokenizer, overlap_tokens),
            None => "",
        };
        assert!(
            chunk.text.starts_with(overlap),
            "chunk {index} must repeat the overlap suffix"
        );
        contents.push(chunk.text[overlap.len()..].to_string());
    }
    contents
}

fn chunk_text(
    body_text: &str,
    tokenizer: TokenizerKind,
    max_tokens: usize,
    overlap_tokens: usize,
) -> Vec<DocumentChunk> {
    super::chunk_document_with_budget(
        1,
        "file_library",
        &document(body_text),
        &budget(tokenizer, max_tokens, overlap_tokens),
    )
    .expect("the token budget should be valid")
}

fn assert_budget_respected(chunks: &[DocumentChunk], tokenizer: TokenizerKind, max_tokens: usize) {
    for chunk in chunks {
        assert!(
            token_count(&chunk.text, tokenizer) <= max_tokens,
            "chunk {} encodes to {} tokens, budget is {max_tokens}",
            chunk.chunk_index,
            token_count(&chunk.text, tokenizer)
        );
    }
}

#[test]
fn token_chunks_respect_the_budget_for_english_text() {
    let body = format!(
        "{}\n\n{}",
        "The quick brown fox jumps over the lazy dog. ".repeat(12),
        "A second paragraph keeps the chunker honest about paragraph joins. ".repeat(12)
    );

    for tokenizer in TOKENIZERS {
        let chunks = chunk_text(&body, tokenizer, 64, 16);
        assert!(chunks.len() > 1, "the budget must split the document");
        assert_budget_respected(&chunks, tokenizer, 64);
    }
}

#[test]
fn token_chunks_respect_the_budget_for_chinese_text() {
    let body = format!(
        "{}\n\n{}",
        "这是一个用于测试分块的中文段落。".repeat(12),
        "第二个段落同样需要保持在预算之内。".repeat(12)
    );

    for tokenizer in TOKENIZERS {
        let chunks = chunk_text(&body, tokenizer, 64, 16);
        assert!(chunks.len() > 1);
        assert_budget_respected(&chunks, tokenizer, 64);
    }
}

#[test]
fn token_chunks_respect_the_budget_for_code() {
    let body = format!(
        "{}\n\n{}",
        "fn main() { let value = compute(42); println!(\"{value}\"); } ".repeat(8),
        "let other = vec![1, 2, 3]; // trailing comment".repeat(8)
    );

    for tokenizer in TOKENIZERS {
        let chunks = chunk_text(&body, tokenizer, 64, 16);
        assert!(chunks.len() > 1);
        assert_budget_respected(&chunks, tokenizer, 64);
    }
}

#[test]
fn token_chunks_respect_the_budget_for_emoji() {
    let body = format!("{}\n\n{}", "🚀🧪🧭".repeat(20), "✅🎯🌍".repeat(20));

    for tokenizer in TOKENIZERS {
        let chunks = chunk_text(&body, tokenizer, 48, 12);
        assert!(chunks.len() > 1);
        assert_budget_respected(&chunks, tokenizer, 48);
    }
}

#[test]
fn token_chunks_respect_the_budget_for_combining_marks() {
    let body = format!(
        "{}\n\n{}",
        "e\u{0301}a\u{0300}o\u{0302}".repeat(20),
        "na\u{0308}ive".repeat(20)
    );

    for tokenizer in TOKENIZERS {
        let chunks = chunk_text(&body, tokenizer, 48, 12);
        assert!(chunks.len() > 1);
        assert_budget_respected(&chunks, tokenizer, 48);
    }
}

#[test]
fn token_chunks_cover_the_normalized_source_in_order() {
    let body = format!(
        "{}\n\n{}\n\n{}",
        "Alpha paragraph with enough words to fill a small token budget.".repeat(4),
        "Bravo paragraph keeps the order of the document.".repeat(4),
        "Charlie paragraph closes the document.".repeat(4)
    );

    for tokenizer in TOKENIZERS {
        let chunks = chunk_text(&body, tokenizer, 48, 12);
        let contents = contents(&chunks, tokenizer, 12);
        assert_eq!(
            contents.concat(),
            normalized_source(&body),
            "the chunk contents must cover the normalized source in order"
        );
    }
}

#[test]
fn token_chunks_cover_the_normalized_source_without_overlap() {
    let body = format!(
        "{}\n\n{}",
        "Delta paragraph without any repeated content.".repeat(6),
        "Echo paragraph also stays inside the budget.".repeat(6)
    );

    for tokenizer in TOKENIZERS {
        let chunks = chunk_text(&body, tokenizer, 48, 0);
        let contents = contents(&chunks, tokenizer, 0);
        assert_eq!(contents.concat(), normalized_source(&body));
        assert_budget_respected(&chunks, tokenizer, 48);
    }
}

#[test]
fn token_overlap_is_a_bounded_suffix_of_the_previous_chunk() {
    let body = "Foxtrot paragraph repeats a bounded suffix of the previous chunk.".repeat(12);

    for tokenizer in TOKENIZERS {
        let chunks = chunk_text(&body, tokenizer, 48, 12);
        let contents = contents(&chunks, tokenizer, 12);
        for index in 1..chunks.len() {
            let overlap = token_suffix(&contents[index - 1], tokenizer, 12);
            assert!(
                chunks[index].text.starts_with(overlap),
                "chunk {index} must start with the overlap suffix"
            );
            assert!(chunks[index - 1].text.ends_with(overlap));
            assert!(token_count(overlap, tokenizer) <= 12);
        }
    }
}

#[test]
fn literal_special_token_text_is_preserved() {
    let body = "The literal <|endoftext|> marker stays plain text.\n\nAnd <|im_start|> too.";

    for tokenizer in TOKENIZERS {
        let chunks = chunk_text(body, tokenizer, 16, 4);
        let contents = contents(&chunks, tokenizer, 4);
        assert_eq!(contents.concat(), normalized_source(body));
        assert!(contents.concat().contains("<|endoftext|>"));
        assert!(contents.concat().contains("<|im_start|>"));
    }
}

#[test]
fn paragraphs_are_joined_before_a_paragraph_is_split() {
    let body = "First paragraph.\n\nSecond paragraph.\n\nThird paragraph.";

    for tokenizer in TOKENIZERS {
        let chunks = chunk_text(body, tokenizer, 64, 8);
        assert_eq!(chunks.len(), 1, "short paragraphs stay in one chunk");
        assert_eq!(chunks[0].text, normalized_source(body));
    }
}

#[test]
fn oversized_paragraphs_are_split_without_losing_text() {
    let body = "Golf paragraph with no paragraph boundary at all ".repeat(40);

    for tokenizer in TOKENIZERS {
        let chunks = chunk_text(&body, tokenizer, 32, 8);
        assert!(chunks.len() > 1);
        assert_budget_respected(&chunks, tokenizer, 32);
        let contents = contents(&chunks, tokenizer, 8);
        assert_eq!(contents.concat(), normalized_source(&body));
    }
}

#[test]
fn tiny_token_budgets_still_cover_the_source() {
    // A one- or two-token budget is valid, so the chunker must keep advancing
    // after the first chunk: the overlap is optional and shrinks instead of
    // consuming the whole content budget.
    for tokenizer in TOKENIZERS {
        for (body, max_tokens) in [("a b", 1), ("a b", 2), ("a\n\nb", 1)] {
            let chunks = chunk_text(body, tokenizer, max_tokens, 0);
            assert!(
                !chunks.is_empty(),
                "{body:?} at {max_tokens} tokens must still emit a chunk"
            );
            assert_budget_respected(&chunks, tokenizer, max_tokens);
            assert_eq!(
                contents(&chunks, tokenizer, 0).concat(),
                normalized_source(body),
                "{body:?} at {max_tokens} tokens must cover the source in order"
            );
        }

        // The window boundary: a paragraph that fills the pending window
        // leaves no room for the paragraph separator, so the separator waits
        // for the next output instead of overflowing the window. The source
        // must still be covered exactly, in order, at a one-token budget.
        for prefix in [38usize, 39, 40] {
            let body = format!("{}\n\nb", "a".repeat(prefix));
            let chunks = chunk_text(&body, tokenizer, 1, 0);
            assert!(
                !chunks.is_empty(),
                "{prefix} characters before the separator must still emit a chunk"
            );
            assert_budget_respected(&chunks, tokenizer, 1);
            assert_eq!(
                contents(&chunks, tokenizer, 0).concat(),
                normalized_source(&body),
                "{prefix} characters before the separator must be covered in order"
            );
            assert!(
                chunks.last().unwrap().text.ends_with('b'),
                "{prefix} characters before the separator must keep the last paragraph"
            );
        }

        // Near-maximum overlap shrinks rather than exhausting the budget: the
        // content still advances from the first to the last word.
        let body = "alpha beta gamma delta epsilon";
        let chunks = chunk_text(body, tokenizer, 4, 3);
        assert!(chunks.len() > 1, "the tiny budget must split the sentence");
        assert_budget_respected(&chunks, tokenizer, 4);
        assert!(chunks[0].text.starts_with("alpha"));
        assert!(chunks.last().unwrap().text.ends_with("epsilon"));

        // A paragraph of tens of words under a near-maximum overlap: the
        // content budget is a single token, so the chunker must advance
        // through the whole paragraph, repeat the bounded overlap, and cover
        // every word in order. The paragraph is never materialized as a list
        // of pieces, so its length does not change the work per chunk.
        let body = "alpha beta gamma delta epsilon zeta eta theta iota kappa";
        let chunks = chunk_text(body, tokenizer, 4, 3);
        assert!(
            chunks.len() > 1,
            "the near-maximum overlap must split the paragraph"
        );
        assert_budget_respected(&chunks, tokenizer, 4);
        assert_eq!(
            contents(&chunks, tokenizer, 3).concat(),
            normalized_source(body),
            "the paragraph must be covered in order under a near-maximum overlap"
        );
        assert!(
            chunks.last().unwrap().text.ends_with("kappa"),
            "the last word must survive the near-maximum overlap"
        );
    }

    // Representative CJK content with a token budget and near-maximum overlap.
    for tokenizer in TOKENIZERS {
        let body = "中文测试内容分块示例";
        let chunks = chunk_text(body, tokenizer, 4, 2);
        assert!(chunks.len() > 1, "the CJK source must split at four tokens");
        assert_budget_respected(&chunks, tokenizer, 4);
        assert_eq!(
            contents(&chunks, tokenizer, 2).concat(),
            normalized_source(body)
        );
    }
}

#[test]
fn a_token_budget_that_cannot_hold_one_character_fails_without_panic() {
    let body = "🚀";

    for tokenizer in TOKENIZERS {
        let result = super::chunk_document_with_budget(
            1,
            "file_library",
            &document(body),
            &budget(tokenizer, 1, 0),
        );
        match result {
            Ok(chunks) => panic!("a single emoji must not fit into a one token budget: {chunks:?}"),
            Err(error) => assert!(
                matches!(
                    error,
                    ChunkingError::CharacterExceedsBudget { .. }
                        | ChunkingError::TokenBudgetExceeded { .. }
                ),
                "unexpected error: {error}"
            ),
        }
    }
}

#[test]
fn invalid_token_budgets_are_rejected() {
    let document = document("text");

    let overlapping = super::chunk_document_with_budget(
        1,
        "file_library",
        &document,
        &budget(TokenizerKind::Cl100kBase, 10, 10),
    );
    assert!(matches!(
        overlapping,
        Err(ChunkingError::InvalidTokenBudget { .. })
    ));

    let empty = super::chunk_document_with_budget(
        1,
        "file_library",
        &document,
        &budget(TokenizerKind::Cl100kBase, 0, 0),
    );
    assert!(matches!(
        empty,
        Err(ChunkingError::InvalidTokenBudget { .. })
    ));
}

#[test]
fn token_chunk_ids_are_deterministic() {
    let body = "Hotel paragraph that produces more than one chunk.".repeat(12);
    let document = document(&body);

    for tokenizer in TOKENIZERS {
        let budget = budget(tokenizer, 32, 8);
        let first = super::chunk_document_with_budget(1, "file_library", &document, &budget)
            .expect("the token budget should be valid");
        let second = super::chunk_document_with_budget(1, "file_library", &document, &budget)
            .expect("the token budget should be valid");

        assert_eq!(
            first.iter().map(|chunk| chunk.id).collect::<Vec<_>>(),
            second.iter().map(|chunk| chunk.id).collect::<Vec<_>>()
        );
        assert_eq!(
            first
                .iter()
                .map(|chunk| chunk.chunk_index)
                .collect::<Vec<_>>(),
            (0..first.len() as i32).collect::<Vec<_>>()
        );
        assert!(first.iter().all(|chunk| chunk.document_id == 1));
    }
}

#[test]
fn token_budget_identity_is_deterministic() {
    let budget = budget(TokenizerKind::O200kBase, 1024, 128);
    assert_eq!(budget.identity(), "tokens:o200k_base:1024:128");
    assert_eq!(
        budget.identity(),
        ChunkBudget::tokens(TokenizerKind::O200kBase, 1024, 128).identity()
    );
    assert_ne!(
        budget.identity(),
        ChunkBudget::tokens(TokenizerKind::Cl100kBase, 1024, 128).identity()
    );
    assert_eq!(
        ChunkBudget::from(&super::ChunkingConfig {
            max_chars: 1200,
            overlap_chars: 200
        })
        .identity(),
        "chars:1200:200"
    );
}

/// The prefix is counted with the chunk text in one exact encoding, so a
/// nonadditive token that spans their boundary is not double-counted.
#[test]
fn the_input_envelope_shortens_token_chunks_and_preserves_the_source() {
    let prefix = "system:";
    let sample: String = (0..40).map(|index| format!("word{index} ")).collect();
    let mut policy = context69_contracts::settings::EmbeddingInputPolicy::default();
    policy.document_server_prefix = prefix.to_string();
    policy.max_input_tokens = Some(token_count(
        &format!("{prefix}{sample}"),
        TokenizerKind::Cl100kBase,
    ));
    let mut budget = budget(TokenizerKind::Cl100kBase, 64, 16);
    budget.input_policy = Some(policy.clone());
    let runtime = super::ChunkingRuntime::new(budget.clone());
    let resolved = runtime
        .resolve(&budget)
        .expect("resolve the pinned envelope");

    let body: String = (0..400).map(|index| format!("segment{index} ")).collect();
    let chunks = resolved
        .chunk_document(1, "file_library", &document(&body))
        .expect("the envelope chunks the source");
    assert!(chunks.len() > 1, "the envelope must split the document");
    let envelope = resolved.input_policy().expect("the policy is pinned");
    let cap = policy.max_input_tokens.expect("the cap is set");
    for chunk in &chunks {
        assert!(
            envelope.count_input(prefix, &chunk.text) <= cap,
            "every emitted chunk fits the complete envelope"
        );
    }
    let novel = contents(&chunks, TokenizerKind::Cl100kBase, 16);
    assert_eq!(novel.concat(), normalized_source(&body));
}

/// The emitted overlap is the one the cut actually chose, not the previous
/// overlap that the local search shortened to make room.
#[test]
fn char_envelope_emits_the_selected_overlap_and_content_together() {
    let mut policy = context69_contracts::settings::EmbeddingInputPolicy::default();
    policy.max_input_tokens = Some(1);
    let mut budget = ChunkBudget::chars(super::ChunkingConfig {
        max_chars: 5,
        overlap_chars: 4,
    });
    budget.input_policy = Some(policy.clone());
    let runtime = super::ChunkingRuntime::new(budget.clone());
    let resolved = runtime
        .resolve(&budget)
        .expect("resolve the pinned envelope");
    let chunks = resolved
        .chunk_document(1, "file_library", &document("hello world"))
        .expect("the envelope chunks the source");

    assert_eq!(
        chunks[1].text, " ",
        "the shortened overlap must not be emitted as if it still fit"
    );
    let envelope = resolved.input_policy().expect("the policy is pinned");
    let mut source = String::new();
    for (index, chunk) in chunks.iter().enumerate() {
        assert!(
            envelope.count_input(&policy.document_server_prefix, &chunk.text) <= 1,
            "every emitted chunk fits the one-token envelope"
        );
        let overlap = if index == 0 {
            ""
        } else {
            let previous = chunks[index - 1].text.as_str();
            let previous_chars: Vec<char> = previous.chars().collect();
            let chunk_chars: Vec<char> = chunk.text.chars().collect();
            let max_overlap = 4.min(previous_chars.len()).min(chunk_chars.len());
            let mut selected = 0usize;
            for length in 1..=max_overlap {
                let previous_suffix = &previous_chars[previous_chars.len() - length..];
                let chunk_prefix = &chunk_chars[..length];
                if previous_suffix == chunk_prefix {
                    selected = length;
                }
            }
            let prefix_bytes: usize = chunk_chars[..selected].iter().map(|c| c.len_utf8()).sum();
            &chunk.text[..prefix_bytes]
        };
        assert!(chunk.text.starts_with(overlap));
        source.push_str(&chunk.text[overlap.len()..]);
    }
    assert_eq!(source, "hello world");
}

/// A valid positive prefix is emitted even when a longer prefix and the next
/// shorter prefix both exceed the cap, because token counts are not monotonic.
#[test]
fn nonmonotonic_envelope_prefix_is_not_refused() {
    let mut policy = context69_contracts::settings::EmbeddingInputPolicy::default();
    policy.document_server_prefix = "Ini".to_string();
    policy.max_input_tokens = Some(1);

    let mut char_budget = ChunkBudget::chars(super::ChunkingConfig {
        max_chars: 4,
        overlap_chars: 0,
    });
    char_budget.input_policy = Some(policy.clone());
    let char_runtime = super::ChunkingRuntime::new(char_budget.clone());
    let char_resolved = char_runtime
        .resolve(&char_budget)
        .expect("resolve the char envelope");
    let char_source = document("ciox");
    let mut char_chunks = char_resolved
        .chunk_document_iter(1, "file_library", &char_source, None)
        .expect("the char envelope iterates");
    let first_char = char_chunks
        .next()
        .expect("the positive prefix is emitted")
        .expect("the positive prefix fits");
    assert_eq!(first_char.text, "cio");
    assert!(
        char_resolved
            .input_policy()
            .expect("the policy is pinned")
            .count_input("Ini", &first_char.text)
            <= 1
    );
    assert!(matches!(
        char_chunks.next(),
        Some(Err(ChunkingError::InputEnvelopeExceeded { .. }))
    ));

    let mut token_budget = budget(TokenizerKind::Cl100kBase, 8, 0);
    token_budget.input_policy = Some(policy.clone());
    let token_runtime = super::ChunkingRuntime::new(token_budget.clone());
    let token_resolved = token_runtime
        .resolve(&token_budget)
        .expect("resolve the token envelope");
    let token_source = document("ciox");
    let mut token_chunks = token_resolved
        .chunk_document_iter(1, "file_library", &token_source, None)
        .expect("the token envelope iterates");
    let first_token = token_chunks
        .next()
        .expect("the positive prefix is emitted")
        .expect("the positive prefix fits");
    assert_eq!(first_token.text, "cio");
    assert!(
        token_resolved
            .input_policy()
            .expect("the policy is pinned")
            .count_input("Ini", &first_token.text)
            <= 1
    );
    assert!(matches!(
        token_chunks.next(),
        Some(Err(ChunkingError::InputEnvelopeExceeded { .. }))
    ));
}
