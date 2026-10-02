//! Exact code content contracts (issue #681 work unit 3B2 and phase 5C).
//!
//! Covers the wire shape of the manifest, chunk, lexical-hit, and bounded
//! line-window content contracts: an entry points at stored bytes without
//! carrying them, a chunk preserves its source text verbatim with inclusive line
//! anchors, a lexical hit carries the repository/ref/commit/path/line
//! provenance a code result must be checkable against, and a content page
//! carries exactly the window text it promised. The migration and query
//! invariants these contracts depend on are asserted next to the persistence
//! layer that relies on them, in `src/db/git_repositories/schema_tests.rs`.

use chrono::{DateTime, Utc};
use context69_contracts_core::Visibility;
use context69_contracts_core::pagination::CursorPagination;
use context69_contracts_sources::git_files::{
    GIT_REPOSITORY_FILE_PATH_MAX_CHARS, GitCodeChunk, GitCodeLexicalHit, GitCodeMatchKind,
    GitRepositoryFile, GitRepositoryFileContentQuery, GitRepositoryFileContentResponse,
    MAX_GIT_CONTENT_CURSOR_MAX_CHARS, MAX_GIT_CONTENT_WINDOW_BYTES, MAX_GIT_CONTENT_WINDOW_LINES,
};
use context69_contracts_sources::git_repositories::{GitCommitCheckpoint, GitIndexStatus};
use schemars::schema_for;
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

#[test]
fn content_response_carries_exact_window_text_and_nothing_else() {
    let response = sample_content_response();
    let encoded = to_value(&response).expect("serialize content response");
    let object = encoded.as_object().expect("content object");
    let mut keys = object.keys().map(String::as_str).collect::<Vec<_>>();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "byte_count",
            "checkpoint",
            "commit_sha",
            "end_line",
            "excluded_file_count",
            "file",
            "file_count",
            "generation_key",
            "generation_number",
            "index_status",
            "pagination",
            "ref_name",
            "repository_key",
            "start_line",
            "text",
            "total_bytes",
        ],
        "the content response is the exact window text plus the entry and the \
         generation provenance: no blob, provider, or connection field"
    );
    assert_eq!(object["start_line"], json!(10));
    assert_eq!(object["end_line"], json!(14));
    assert_eq!(object["text"], json!("fn window() {\n    1.0;\n"));
    assert_eq!(
        object["byte_count"].as_i64().expect("a count"),
        response.text.len() as i64,
        "a caller can verify the text it received against the reported count"
    );
    assert_eq!(object["file"]["path"], json!("src/main.rs"));

    // The only content is the window's own text. A forbidden field would appear
    // in the serialized form, so one substring sweep covers the whole response:
    // no acquisition blob, provider blob id, secret reference, or byte-range or
    // chunk plumbing.
    let serialized = serde_json::to_string(&response).expect("serialize content response");
    for forbidden in [
        "cccccccccccccccccccccccccccccccccccccccc",
        "internal/secret",
        "provider_blob_sha",
        "canonical_url",
        "connection_key",
        "chunk_text",
        "content",
        "blob",
        "chunks",
        "start_byte",
        "end_byte",
        "secret",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "the content response must not carry {forbidden}: {serialized}"
        );
    }
}

#[test]
fn content_response_continuation_is_truthful() {
    let mut continued = sample_content_response();
    continued.pagination = CursorPagination::new(Some("4".to_string()), true);
    let encoded = to_value(&continued).expect("serialize continued page");
    assert_eq!(encoded["pagination"]["has_more"], json!(true));
    assert_eq!(encoded["pagination"]["next_cursor"], json!("4"));
    continued
        .pagination
        .validate_continuation()
        .expect("has_more must carry its continuation token");

    let terminal = sample_content_response();
    let encoded = to_value(&terminal).expect("serialize terminal page");
    assert_eq!(encoded["pagination"]["has_more"], json!(false));
    assert!(
        encoded["pagination"]
            .as_object()
            .expect("pagination object")
            .get("next_cursor")
            .is_none(),
        "a page with no more text must omit next_cursor rather than send an empty one"
    );
}

#[test]
fn content_query_declares_bounded_path_line_and_cursor_inputs() {
    let query = schema_for!(GitRepositoryFileContentQuery);
    let properties = query
        .get("properties")
        .and_then(|properties| properties.as_object())
        .expect("query properties");
    let mut keys = properties.keys().map(String::as_str).collect::<Vec<_>>();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec!["cursor", "end_line", "path", "start_line"],
        "the read takes a path, a line window, and an optional continuation"
    );
    assert_eq!(
        properties
            .get("path")
            .and_then(|path| path.get("maxLength"))
            .and_then(|value| value.as_u64()),
        Some(GIT_REPOSITORY_FILE_PATH_MAX_CHARS as u64)
    );
    for line in ["start_line", "end_line"] {
        let property = properties.get(line).expect("line property");
        assert_eq!(
            property.get("minimum").and_then(|value| value.as_u64()),
            Some(1),
            "{line} is a positive 1-based source line"
        );
        assert!(
            property.get("maximum").is_none(),
            "{line} is bounded by the span rule, not by a fixed line count"
        );
    }
    let cursor = properties.get("cursor").expect("cursor property");
    assert_eq!(
        cursor.get("maxLength").and_then(|value| value.as_u64()),
        Some(MAX_GIT_CONTENT_CURSOR_MAX_CHARS as u64),
        "the continuation is a bounded server-issued token"
    );
    let required = query
        .get("required")
        .and_then(|required| required.as_array())
        .expect("required fields");
    let required: Vec<&str> = required.iter().filter_map(|value| value.as_str()).collect();
    for field in ["path", "start_line", "end_line"] {
        assert!(required.contains(&field), "{field} is required");
    }
    assert!(
        !required.contains(&"cursor"),
        "an absent cursor starts the window, so it is not required"
    );

    // The span and byte bounds are enforced at runtime rather than by the
    // schema, so they are named constants a caller can reason about.
    assert_eq!(MAX_GIT_CONTENT_WINDOW_LINES, 400, "the line window");
    assert_eq!(MAX_GIT_CONTENT_WINDOW_BYTES, 64 * 1024, "the byte cap");
    let decoded: GitRepositoryFileContentQuery =
        from_value(json!({ "path": "src/main.rs", "start_line": 10, "end_line": 14 }))
            .expect("decode a window query");
    assert_eq!(decoded.start_line, 10);
    assert_eq!(decoded.end_line, 14);
    assert_eq!(decoded.cursor, None);
}

#[test]
fn content_response_embeds_the_unchanged_manifest_entry() {
    let response = sample_content_response();
    let schema = schema_for!(GitRepositoryFileContentResponse);
    let embedded = schema
        .pointer("/properties/file")
        .expect("the content response carries a file property");
    let entry = embedded
        .get("allOf")
        .and_then(|all_of| all_of[0].get("$ref"))
        .or_else(|| embedded.get("$ref"))
        .and_then(|reference| reference.as_str())
        .expect("the content response carries the unchanged entry projection");
    assert_eq!(entry, "#/$defs/GitRepositoryFile");
    assert_eq!(
        response.file,
        manifest_entry(),
        "a content read carries the same entry a manifest page or detail read returns"
    );
}

fn manifest_entry() -> GitRepositoryFile {
    GitRepositoryFile {
        file_key: Uuid::parse_str("018f9f40-1111-7000-8000-0000000000c1").expect("uuid"),
        generation_key: Uuid::parse_str("018f9f40-2222-7000-8000-0000000000c2").expect("uuid"),
        repository_key: Uuid::parse_str("018f9f3a-2c1b-7c4d-8e5f-6a7b8c9d0e1f").expect("uuid"),
        path: "src/main.rs".to_string(),
        language: "rust".to_string(),
        byte_count: 512,
        line_count: 20,
        created_at: timestamp(),
    }
}

fn sample_content_response() -> GitRepositoryFileContentResponse {
    GitRepositoryFileContentResponse {
        repository_key: Uuid::parse_str("018f9f3a-2c1b-7c4d-8e5f-6a7b8c9d0e1f").expect("uuid"),
        generation_key: Uuid::parse_str("018f9f40-2222-7000-8000-0000000000c2").expect("uuid"),
        generation_number: 4,
        ref_name: "refs/heads/main".to_string(),
        commit_sha: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        index_status: GitIndexStatus::Stale,
        checkpoint: GitCommitCheckpoint {
            target_commit_sha: Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string()),
            indexed_commit_sha: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string()),
            indexed_at: Some(timestamp()),
            checkpoint_updated_at: Some(timestamp()),
        },
        file_count: 120,
        excluded_file_count: 3,
        total_bytes: 4096,
        file: manifest_entry(),
        start_line: 10,
        end_line: 14,
        text: "fn window() {\n    1.0;\n".to_string(),
        byte_count: 23,
        pagination: CursorPagination::terminal(),
    }
}
