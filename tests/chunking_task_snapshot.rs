//! Pinned chunk-budget payload units.
//!
//! Pinning is a payload transformation, not a replacement: the client keys are
//! preserved and a malformed snapshot fails closed.

use context69::chunking::{ChunkBudget, ChunkingConfig, payload_with_chunk_budget};
use serde_json::{Value, json};

fn chars_budget(max_chars: usize, overlap_chars: usize) -> ChunkBudget {
    ChunkBudget::chars(ChunkingConfig {
        max_chars,
        overlap_chars,
    })
}

#[test]
fn a_pinned_payload_keeps_the_client_keys_and_a_malformed_one_is_refused() {
    let payload = json!({"external_id": "keep-me", "options": {"ocr": true}});
    let pinned = payload_with_chunk_budget(&payload, &chars_budget(1000, 100));
    assert_eq!(
        pinned.get("external_id").and_then(Value::as_str),
        Some("keep-me")
    );
    assert_eq!(
        pinned
            .get("options")
            .and_then(|options| options.get("ocr"))
            .and_then(Value::as_bool),
        Some(true)
    );

    let malformed = json!({"chunk_budget": {"mode": "tokens"}});
    assert!(
        context69::chunking::parse_chunk_budget(&malformed).is_err(),
        "a malformed snapshot must fail closed"
    );
}
