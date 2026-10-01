//! Exact code content contracts (issue #681 work unit 3B2).
//!
//! Covers the wire shape of the manifest, chunk, and lexical-hit contracts:
//! a manifest entry points at stored bytes without carrying them, a chunk
//! preserves its source text verbatim with inclusive line anchors, and a
//! lexical hit carries the repository/ref/commit/path/line provenance a code
//! result must be checkable against. The migration and query invariants these
//! contracts depend on are asserted next to the persistence layer that relies
//! on them, in `src/db/git_repositories/schema_tests.rs`.

use chrono::{DateTime, Utc};
use context69_contracts_core::Visibility;
use context69_contracts_sources::git_files::{
    GitCodeChunk, GitCodeLexicalHit, GitCodeMatchKind, GitRepositoryFile,
};
use serde_json::{from_value, json, to_value};
use uuid::Uuid;

fn timestamp() -> DateTime<Utc> {
    "2026-10-01T05:27:06Z".parse().expect("timestamp")
}

fn keys(value: &serde_json::Value) -> Vec<&str> {
    let mut keys = value
        .as_object()
        .expect("contract object")
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    keys.sort_unstable();
    keys
}

#[test]
fn match_kinds_are_stable_on_the_wire() {
    assert_eq!(
        to_value([
            GitCodeMatchKind::PathExact,
            GitCodeMatchKind::PathPhrase,
            GitCodeMatchKind::ChunkPhrase,
            GitCodeMatchKind::ChunkTerms,
        ])
        .expect("serialize match kinds"),
        json!(["path_exact", "path_phrase", "chunk_phrase", "chunk_terms"])
    );
    assert_eq!(GitCodeMatchKind::PathExact.as_str(), "path_exact");
    assert_eq!(GitCodeMatchKind::ChunkPhrase.as_str(), "chunk_phrase");
    assert_eq!(GitCodeMatchKind::ChunkTerms.as_str(), "chunk_terms");
}

#[test]
fn manifest_entry_carries_identity_without_content() {
    let file = GitRepositoryFile {
        file_key: Uuid::parse_str("018f9f40-1111-7000-8000-0000000000c1").expect("uuid"),
        generation_key: Uuid::parse_str("018f9f40-2222-7000-8000-0000000000c2").expect("uuid"),
        repository_key: Uuid::parse_str("018f9f3a-2c1b-7c4d-8e5f-6a7b8c9d0e1f").expect("uuid"),
        path: "src/db/git_repositories/files.rs".to_string(),
        language: "rust".to_string(),
        byte_count: 4096,
        line_count: 120,
        created_at: timestamp(),
    };
    let encoded = to_value(&file).expect("serialize manifest entry");
    assert_eq!(encoded["path"], json!("src/db/git_repositories/files.rs"));
    assert_eq!(encoded["language"], json!("rust"));
    assert_eq!(encoded["line_count"], json!(120));
    assert_eq!(
        keys(&encoded),
        vec![
            "byte_count",
            "created_at",
            "file_key",
            "generation_key",
            "language",
            "line_count",
            "path",
            "repository_key",
        ],
        "a manifest entry points at stored bytes; the bytes themselves are \
         deduplicated per generation and never travel with the row"
    );

    let decoded: GitRepositoryFile = from_value(encoded).expect("deserialize manifest entry");
    assert_eq!(decoded, file);
}

#[test]
fn chunk_preserves_exact_source_text_with_inclusive_lines() {
    // Mixed line endings, tabs, trailing spaces, and punctuation: the contract
    // must hand back exactly what was stored so a citation can be quoted.
    let text = "fn main() {\r\n\tlet x = 100%;\t \n}\n";
    let chunk = GitCodeChunk {
        chunk_key: Uuid::parse_str("018f9f40-3333-7000-8000-0000000000c3").expect("uuid"),
        file_key: Uuid::parse_str("018f9f40-1111-7000-8000-0000000000c1").expect("uuid"),
        generation_key: Uuid::parse_str("018f9f40-2222-7000-8000-0000000000c2").expect("uuid"),
        chunk_index: 0,
        start_line: 1,
        end_line: 3,
        text: text.to_string(),
        created_at: timestamp(),
    };
    let encoded = to_value(&chunk).expect("serialize chunk");
    assert_eq!(encoded["text"], json!(text));
    assert_eq!(encoded["start_line"], json!(1));
    assert_eq!(encoded["end_line"], json!(3));
    assert_eq!(encoded["chunk_index"], json!(0));

    let decoded: GitCodeChunk = from_value(encoded).expect("deserialize chunk");
    assert_eq!(decoded.text, text, "chunk text is returned verbatim");
    assert_eq!(decoded.start_line, 1);
    assert_eq!(decoded.end_line, 3);
    assert_eq!(decoded, chunk);
}

#[test]
fn lexical_hit_carries_repository_ref_commit_path_and_lines() {
    let hit = GitCodeLexicalHit {
        repository_key: Uuid::parse_str("018f9f3a-2c1b-7c4d-8e5f-6a7b8c9d0e1f").expect("uuid"),
        generation_key: Uuid::parse_str("018f9f40-2222-7000-8000-0000000000c2").expect("uuid"),
        generation_number: 4,
        ref_name: "refs/heads/main".to_string(),
        commit_sha: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
        visibility: Visibility::Private,
        file_key: Uuid::parse_str("018f9f40-1111-7000-8000-0000000000c1").expect("uuid"),
        path: "src/services/git_repository/code_text.rs".to_string(),
        language: "rust".to_string(),
        chunk_key: Uuid::parse_str("018f9f40-3333-7000-8000-0000000000c3").expect("uuid"),
        chunk_index: 2,
        start_line: 41,
        end_line: 57,
        text: "chunk_code_text(text, bounds)\n".to_string(),
        score: 1.1,
        matched: GitCodeMatchKind::ChunkPhrase,
    };
    let encoded = to_value(&hit).expect("serialize lexical hit");
    assert_eq!(encoded["visibility"], json!("private"));
    assert_eq!(encoded["matched"], json!("chunk_phrase"));
    assert_eq!(encoded["commit_sha"], json!(hit.commit_sha));
    assert_eq!(encoded["ref_name"], json!("refs/heads/main"));
    assert_eq!(encoded["generation_number"], json!(4));
    assert_eq!(encoded["start_line"], json!(41));
    assert_eq!(encoded["end_line"], json!(57));
    assert_eq!(encoded["path"], json!(hit.path));

    let decoded: GitCodeLexicalHit = from_value(encoded).expect("deserialize lexical hit");
    assert_eq!(decoded, hit);
}

#[test]
fn lexical_hit_decodes_every_match_kind() {
    for (encoded_kind, expected) in [
        ("path_exact", GitCodeMatchKind::PathExact),
        ("path_phrase", GitCodeMatchKind::PathPhrase),
        ("chunk_phrase", GitCodeMatchKind::ChunkPhrase),
        ("chunk_terms", GitCodeMatchKind::ChunkTerms),
    ] {
        let hit: GitCodeLexicalHit = from_value(json!({
            "repository_key": "018f9f3a-2c1b-7c4d-8e5f-6a7b8c9d0e1f",
            "generation_key": "018f9f40-2222-7000-8000-0000000000c2",
            "generation_number": 1,
            "ref_name": "refs/heads/main",
            "commit_sha": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "visibility": "public",
            "file_key": "018f9f40-1111-7000-8000-0000000000c1",
            "path": "src/main.rs",
            "language": "rust",
            "chunk_key": "018f9f40-3333-7000-8000-0000000000c3",
            "chunk_index": 0,
            "start_line": 1,
            "end_line": 1,
            "text": "fn main() {}\n",
            "score": 1.2,
            "matched": encoded_kind
        }))
        .unwrap_or_else(|error| panic!("{encoded_kind} hit must decode: {error}"));
        assert_eq!(hit.matched, expected);
    }
}
