//! Exact code content contracts (issue #681 work unit 3B2 and phases 5C, 5D,
//! and 5E).
//!
//! Covers the wire shape of the manifest, chunk, lexical-hit, bounded
//! line-window content, code-search, and generation-comparison contracts: an entry
//! points at stored bytes without carrying them, a chunk preserves its source
//! text verbatim with inclusive line anchors, a lexical hit carries the
//! repository/ref/commit/path/line provenance a code result must be checkable
//! against, a content page carries exactly the window text it promised, and a
//! comparison page names both generations and only safe path metadata. The
//! migration and query invariants these contracts depend on are asserted next to
//! the persistence layer that relies on them, in
//! `src/db/git_repositories/schema_tests.rs`.

use chrono::{DateTime, Utc};
use context69_contracts_core::Visibility;
use context69_contracts_core::pagination::CursorPagination;
use context69_contracts_core::pagination::default_limit;
use context69_contracts_sources::git_files::{
    GIT_CODE_SEARCH_LANGUAGE_MAX_CHARS, GIT_CODE_SEARCH_LIMIT_DEFAULT, GIT_CODE_SEARCH_LIMIT_MAX,
    GIT_CODE_SEARCH_LIMIT_MIN, GIT_CODE_SEARCH_PATH_PREFIX_MAX_CHARS,
    GIT_CODE_SEARCH_QUERY_MAX_CHARS, GIT_REPOSITORY_FILE_PATH_MAX_CHARS, GitCodeChunk,
    GitCodeLexicalHit, GitCodeMatchKind, GitCodeSearchHit, GitCodeSearchQuery,
    GitCodeSearchResponse, GitFileChangeKind, GitRepositoryFile, GitRepositoryFileContentQuery,
    GitRepositoryFileContentResponse, GitRepositoryFileDiff, GitRepositoryFileDiffFile,
    GitRepositoryFileDiffQuery, GitRepositoryFileDiffResponse, MAX_GIT_CONTENT_CURSOR_MAX_CHARS,
    MAX_GIT_CONTENT_WINDOW_BYTES, MAX_GIT_CONTENT_WINDOW_LINES,
};
use context69_contracts_sources::git_repositories::{GitCommitCheckpoint, GitIndexStatus};
use schemars::schema_for;
use serde_json::{from_value, json, to_value};
use uuid::Uuid;

fn timestamp() -> DateTime<Utc> {
    "2026-10-01T05:27:06Z".parse().expect("timestamp")
}

/// One side of a comparison sample: identity, provenance, and coverage.
///
/// The wire contract spells those fields out once per side, so both samples are
/// built from this shared shape instead of restating each side.
struct GitRepositoryGenerationEnvelope {
    generation_key: Uuid,
    generation_number: i64,
    ref_name: String,
    commit_sha: String,
    file_count: i64,
    excluded_file_count: i64,
    total_bytes: i64,
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

#[test]
fn code_search_query_declares_bounded_filters_and_a_documented_default() {
    let schema = schema_for!(GitCodeSearchQuery);
    let properties = schema
        .get("properties")
        .and_then(|properties| properties.as_object())
        .expect("query properties");
    let mut names = properties.keys().map(String::as_str).collect::<Vec<_>>();
    names.sort_unstable();
    assert_eq!(
        names,
        vec!["language", "limit", "path_prefix", "query"],
        "the search takes a term, two optional filters, and a bounded limit"
    );
    let term = properties.get("query").expect("term property");
    assert_eq!(
        term.get("minLength").and_then(|value| value.as_u64()),
        Some(1),
        "a search always names a term"
    );
    assert_eq!(
        term.get("maxLength").and_then(|value| value.as_u64()),
        Some(GIT_CODE_SEARCH_QUERY_MAX_CHARS as u64)
    );
    for (filter, bound) in [
        ("path_prefix", GIT_CODE_SEARCH_PATH_PREFIX_MAX_CHARS),
        ("language", GIT_CODE_SEARCH_LANGUAGE_MAX_CHARS),
    ] {
        assert_eq!(
            properties
                .get(filter)
                .and_then(|value| value.get("maxLength"))
                .and_then(|value| value.as_u64()),
            Some(bound as u64),
            "{filter} is a bounded optional filter"
        );
    }
    let limit = properties.get("limit").expect("limit property");
    assert_eq!(
        limit.get("minimum").and_then(|value| value.as_u64()),
        Some(u64::from(GIT_CODE_SEARCH_LIMIT_MIN))
    );
    assert_eq!(
        limit.get("maximum").and_then(|value| value.as_u64()),
        Some(u64::from(GIT_CODE_SEARCH_LIMIT_MAX))
    );
    let required: Vec<&str> = schema
        .get("required")
        .and_then(|required| required.as_array())
        .expect("required fields")
        .iter()
        .filter_map(|value| value.as_str())
        .collect();
    assert_eq!(required, vec!["query"], "only the term is required");
    assert_eq!(GIT_CODE_SEARCH_LIMIT_MIN, 1);
    assert_eq!(GIT_CODE_SEARCH_LIMIT_MAX, 50);
    assert_eq!(GIT_CODE_SEARCH_QUERY_MAX_CHARS, 200);
    assert_eq!(GIT_CODE_SEARCH_PATH_PREFIX_MAX_CHARS, 512);
    assert_eq!(GIT_CODE_SEARCH_LANGUAGE_MAX_CHARS, 32);

    // A caller that names no limit still gets a bounded page, and the term
    // reaches storage unmodified, so a `%` or `_` stays a literal character.
    let decoded: GitCodeSearchQuery =
        from_value(json!({ "query": "100%_a" })).expect("decode a term-only search");
    assert_eq!(decoded.query, "100%_a", "the term is never rewritten");
    assert_eq!(decoded.limit, GIT_CODE_SEARCH_LIMIT_DEFAULT);
    assert_eq!(decoded.path_prefix, None);
    assert_eq!(decoded.language, None);
}

#[test]
fn a_code_search_hit_repeats_the_stored_provenance_and_text_verbatim() {
    let stored = sample_code_lexical_hit();
    let hit = sample_code_hit();
    assert_eq!(
        keys(&to_value(&hit).expect("serialize the hit")),
        keys(&to_value(&stored).expect("serialize the stored hit")),
        "an HTTP hit repeats the stored hit's provenance field for field"
    );
    let encoded = to_value(&hit).expect("serialize the hit");
    assert_eq!(
        encoded["text"],
        json!("fn window() {\r\n    1.0;\t\n}\n"),
        "the stored chunk text crosses verbatim, CRLF and trailing tab included"
    );
    assert_eq!(
        from_value::<GitCodeSearchHit>(encoded.clone()).expect("round trip"),
        hit
    );
    for (field, value) in [
        ("repository_key", stored.repository_key),
        ("generation_key", stored.generation_key),
        ("file_key", stored.file_key),
        ("chunk_key", stored.chunk_key),
    ] {
        assert_eq!(
            encoded[field],
            to_value(value).expect("serialize the key"),
            "{field}"
        );
    }
    assert_eq!(encoded["start_line"], json!(41));
    assert_eq!(encoded["end_line"], json!(57));
    assert_eq!(encoded["chunk_index"], json!(2));
    assert_eq!(encoded["matched"], json!("chunk_terms"));
    assert_eq!(encoded["visibility"], json!("private"));
    let schema = to_value(schema_for!(GitCodeSearchHit)).expect("schema object");
    let mut required: Vec<&str> = schema["required"]
        .as_array()
        .expect("required fields")
        .iter()
        .filter_map(|value| value.as_str())
        .collect();
    required.sort_unstable();
    assert_eq!(
        required,
        keys(&encoded),
        "every field of a hit is always present: no provenance is optional, and the \
         hit adds no field the stored projection does not already carry"
    );
}

#[test]
fn the_search_response_names_the_serving_generation_and_its_coverage() {
    let response = sample_search_response();
    assert_eq!(
        keys(&to_value(&response).expect("serialize the search")),
        vec![
            "checkpoint",
            "commit_sha",
            "excluded_file_count",
            "file_count",
            "generation_key",
            "generation_number",
            "hits",
            "index_status",
            "ref_name",
            "repository_key",
            "total_bytes",
            "truncated",
        ],
        "the response carries the generation, its coverage, the bounded hits, and the \
         truncation flag — and no continuation token"
    );
    assert_eq!(response.generation_key, response.hits[0].generation_key);
    assert_eq!(response.commit_sha, response.hits[0].commit_sha);
    assert_eq!(response.file_count, 120);
    assert_eq!(response.excluded_file_count, 3);
    assert_eq!(response.total_bytes, 4096);
    let serialized = to_value(&response)
        .expect("serialize the search")
        .to_string();
    for forbidden in [
        "pagination",
        "next_cursor",
        "has_more",
        "blob",
        "connection",
        "credential",
        "secret",
        "token",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "the search response must not carry {forbidden}: {serialized}"
        );
    }

    // An empty but valid search is a truthful result with the same provenance.
    let mut empty = response.clone();
    empty.hits.clear();
    empty.truncated = false;
    let encoded = to_value(&empty).expect("serialize an empty search");
    assert_eq!(encoded["hits"], json!([]));
    assert_eq!(encoded["truncated"], json!(false));
    assert_eq!(
        encoded["generation_key"],
        to_value(response.generation_key).expect("serialize the generation key")
    );
    assert_eq!(encoded["file_count"], json!(120));
    assert_eq!(
        schema_for!(GitCodeSearchResponse)
            .pointer("/properties/truncated/type")
            .and_then(|value| value.as_str()),
        Some("boolean"),
        "truncation is a flag, not a cursor"
    );
}

fn sample_code_lexical_hit() -> GitCodeLexicalHit {
    GitCodeLexicalHit {
        repository_key: Uuid::parse_str("018f9f3a-2c1b-7c4d-8e5f-6a7b8c9d0e1f").expect("uuid"),
        generation_key: Uuid::parse_str("018f9f40-2222-7000-8000-0000000000c2").expect("uuid"),
        generation_number: 7,
        ref_name: "refs/heads/main".to_string(),
        commit_sha: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        visibility: Visibility::Private,
        file_key: Uuid::parse_str("018f9f40-1111-7000-8000-0000000000c1").expect("uuid"),
        path: "src/api/mod.rs".to_string(),
        language: "rust".to_string(),
        chunk_key: Uuid::parse_str("018f9f40-3333-7000-8000-0000000000c3").expect("uuid"),
        chunk_index: 2,
        start_line: 41,
        end_line: 57,
        text: "fn window() {\r\n    1.0;\t\n}\n".to_string(),
        score: 1.5,
        matched: GitCodeMatchKind::ChunkTerms,
    }
}

fn sample_code_hit() -> GitCodeSearchHit {
    let stored = sample_code_lexical_hit();
    GitCodeSearchHit {
        repository_key: stored.repository_key,
        generation_key: stored.generation_key,
        generation_number: stored.generation_number,
        ref_name: stored.ref_name,
        commit_sha: stored.commit_sha,
        visibility: stored.visibility,
        file_key: stored.file_key,
        path: stored.path,
        language: stored.language,
        chunk_key: stored.chunk_key,
        chunk_index: stored.chunk_index,
        start_line: stored.start_line,
        end_line: stored.end_line,
        text: stored.text,
        score: stored.score,
        matched: stored.matched,
    }
}

fn sample_search_response() -> GitCodeSearchResponse {
    let content = sample_content_response();
    GitCodeSearchResponse {
        repository_key: content.repository_key,
        generation_key: content.generation_key,
        generation_number: content.generation_number,
        ref_name: content.ref_name,
        commit_sha: content.commit_sha,
        index_status: content.index_status,
        checkpoint: content.checkpoint,
        file_count: content.file_count,
        excluded_file_count: content.excluded_file_count,
        total_bytes: content.total_bytes,
        hits: vec![sample_code_hit()],
        truncated: true,
    }
}

#[test]
fn a_change_kind_is_stable_on_the_wire_and_fails_closed() {
    assert_eq!(
        to_value([
            GitFileChangeKind::Added,
            GitFileChangeKind::Modified,
            GitFileChangeKind::Deleted
        ])
        .expect("serialize change kinds"),
        json!(["added", "modified", "deleted"])
    );
    for kind in [
        GitFileChangeKind::Added,
        GitFileChangeKind::Modified,
        GitFileChangeKind::Deleted,
    ] {
        assert_eq!(GitFileChangeKind::from_wire(kind.as_str()), Some(kind));
        assert_eq!(
            from_value::<GitFileChangeKind>(json!(kind.as_str())).expect("decode"),
            kind
        );
    }
    // A kind this storage layer never writes cannot become a fourth kind.
    for unknown in ["unchanged", "renamed", "Added", "MODIFIED", ""] {
        assert_eq!(
            GitFileChangeKind::from_wire(unknown),
            None,
            "{unknown:?} is not a change kind"
        );
        assert!(from_value::<GitFileChangeKind>(json!(unknown)).is_err());
    }
}

#[test]
fn the_diff_query_declares_two_optional_generations_and_the_shared_page() {
    let schema = schema_for!(GitRepositoryFileDiffQuery);
    let properties = schema
        .get("properties")
        .and_then(|properties| properties.as_object())
        .expect("query properties");
    let mut names = properties.keys().map(String::as_str).collect::<Vec<_>>();
    names.sort_unstable();
    assert_eq!(
        names,
        vec!["cursor", "from_generation", "limit", "to_generation",],
        "the comparison takes two optional generations and the shared page window"
    );
    let limit = properties.get("limit").expect("limit property");
    assert_eq!(
        limit.get("minimum").and_then(|value| value.as_u64()),
        Some(1),
        "a page always holds at least one change"
    );
    assert_eq!(
        limit.get("maximum").and_then(|value| value.as_u64()),
        Some(100),
        "the page bound is the shared cursor bound"
    );
    for generation in ["from_generation", "to_generation"] {
        let property = properties.get(generation).expect("generation property");
        assert_eq!(
            property.get("format").and_then(|value| value.as_str()),
            Some("uuid"),
            "{generation} names a generation"
        );
        assert!(
            property.get("default").is_none(),
            "{generation} is resolved server-side when it is absent"
        );
    }
    assert!(
        schema.get("required").is_none(),
        "an omitted comparison resolves to the indexed commit against the active \
         generation, so no field is required"
    );

    // The default limit is the shared cursor default rather than a second number,
    // and the decoded query keeps both generations optional.
    let decoded: GitRepositoryFileDiffQuery = from_value(json!({})).expect("decode an empty query");
    assert_eq!(decoded.limit, default_limit());
    assert_eq!(decoded.from_generation, None);
    assert_eq!(decoded.to_generation, None);
    assert_eq!(decoded.cursor, None);
    let named: GitRepositoryFileDiffQuery = from_value(json!({
        "from_generation": "018f9f40-2222-7000-8000-0000000000c2",
        "to_generation": "018f9f40-3333-7000-8000-0000000000c3",
        "limit": 25,
        "cursor": "25"
    }))
    .expect("decode a fully named query");
    assert_eq!(
        named.from_generation,
        Some(sample_diff_from().generation_key)
    );
    assert_eq!(named.to_generation, Some(sample_diff_to().generation_key));
    assert_eq!(named.limit, 25);
    assert_eq!(named.cursor.as_deref(), Some("25"));
}

#[test]
fn a_diff_page_names_both_generations_its_coverage_and_nothing_else() {
    let response = sample_diff_response();
    assert_eq!(
        keys(&to_value(&response).expect("serialize the diff")),
        vec![
            "changes",
            "checkpoint",
            "from_commit_sha",
            "from_excluded_file_count",
            "from_file_count",
            "from_generation_key",
            "from_generation_number",
            "from_ref_name",
            "from_total_bytes",
            "index_status",
            "pagination",
            "repository_key",
            "to_commit_sha",
            "to_excluded_file_count",
            "to_file_count",
            "to_generation_key",
            "to_generation_number",
            "to_ref_name",
            "to_total_bytes",
        ],
        "a comparison page names both generations, both coverage envelopes, the \
         changes, and the continuation — nothing else"
    );
    assert_eq!(
        response.from_generation_key,
        sample_diff_from().generation_key
    );
    assert_eq!(response.to_generation_key, sample_diff_to().generation_key);
    assert_ne!(
        response.from_commit_sha, response.to_commit_sha,
        "each side is attributable to its own pinned commit"
    );
    assert_eq!(
        (response.from_file_count, response.to_file_count),
        (120, 122)
    );
    assert_eq!(
        (
            response.from_excluded_file_count,
            response.to_excluded_file_count
        ),
        (3, 3)
    );
    assert_eq!(
        (response.from_total_bytes, response.to_total_bytes),
        (4096, 4300)
    );

    let encoded = to_value(&response).expect("serialize the diff");
    for (index, changes) in encoded["changes"]
        .as_array()
        .expect("changes array")
        .iter()
        .enumerate()
    {
        let change = changes.as_object().expect("one change object");
        let mut change_keys = change.keys().map(String::as_str).collect::<Vec<_>>();
        change_keys.sort_unstable();
        assert_eq!(change_keys, vec!["after", "before", "change_kind", "path"]);
        // Every side that is present is safe manifest metadata; the absent one is
        // null rather than an empty entry, so a missing side is unambiguous.
        let present = ["before", "after"]
            .into_iter()
            .filter(|side| !change[*side].is_null())
            .count();
        assert!(present >= 1, "change {index} carries at least one side");
        for side in ["before", "after"] {
            let Some(object) = change[side].as_object() else {
                continue;
            };
            let mut side_keys = object.keys().map(String::as_str).collect::<Vec<_>>();
            side_keys.sort_unstable();
            assert_eq!(
                side_keys,
                vec!["byte_count", "file_key", "language", "line_count"],
                "one side is safe manifest metadata: no content address, no text"
            );
        }
    }
    assert!(
        encoded["changes"][0]["before"].is_null(),
        "an added path has no before side"
    );
    assert!(
        encoded["changes"][2]["after"].is_null(),
        "a deleted path has no after side"
    );
    // An added path has no before side and a deleted path no after side, so the
    // kind never leaves the missing side ambiguous.
    let added = &response.changes[0];
    assert_eq!(added.change_kind, GitFileChangeKind::Added);
    assert!(added.before.is_none() && added.after.is_some());
    let deleted = &response.changes[2];
    assert_eq!(deleted.change_kind, GitFileChangeKind::Deleted);
    assert!(deleted.after.is_none() && deleted.before.is_some());

    let serialized = encoded.to_string();
    for forbidden in [
        "provider_blob_sha",
        "blob_sha",
        "content",
        "chunk",
        "text",
        "hunk",
        "symbol",
        "secret",
        "credential",
        "connection",
        "raw",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "the comparison page must not carry {forbidden}: {serialized}"
        );
    }
    // Unchanged paths are omitted, so a comparison of two generations that hold
    // the same bytes is an empty, terminal page rather than a full manifest.
    let mut unchanged = response.clone();
    unchanged.changes.clear();
    unchanged.pagination = CursorPagination::terminal();
    let empty = to_value(&unchanged).expect("serialize an empty comparison");
    assert_eq!(empty["changes"], json!([]));
    assert_eq!(empty["pagination"]["has_more"], json!(false));
    assert!(
        empty["pagination"]
            .as_object()
            .expect("pagination object")
            .get("next_cursor")
            .is_none(),
        "a complete page omits its continuation token"
    );
    assert_eq!(
        empty["from_generation_key"],
        to_value(response.from_generation_key).expect("serialize the key")
    );
    assert_eq!(empty["to_file_count"], json!(122));
}

fn sample_diff_from() -> GitRepositoryGenerationEnvelope {
    GitRepositoryGenerationEnvelope {
        generation_key: Uuid::parse_str("018f9f40-2222-7000-8000-0000000000c2").expect("uuid"),
        generation_number: 7,
        ref_name: "refs/heads/main".to_string(),
        commit_sha: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        file_count: 120,
        excluded_file_count: 3,
        total_bytes: 4096,
    }
}

fn sample_diff_to() -> GitRepositoryGenerationEnvelope {
    GitRepositoryGenerationEnvelope {
        generation_key: Uuid::parse_str("018f9f40-3333-7000-8000-0000000000c3").expect("uuid"),
        generation_number: 8,
        ref_name: "refs/heads/main".to_string(),
        commit_sha: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
        file_count: 122,
        excluded_file_count: 3,
        total_bytes: 4300,
    }
}

fn sample_diff_file(ordinal: &str) -> GitRepositoryFileDiffFile {
    GitRepositoryFileDiffFile {
        file_key: Uuid::parse_str(ordinal).expect("uuid"),
        language: "rust".to_string(),
        byte_count: 512,
        line_count: 20,
    }
}

fn sample_diff_response() -> GitRepositoryFileDiffResponse {
    let content = sample_content_response();
    let from = sample_diff_from();
    let to = sample_diff_to();
    GitRepositoryFileDiffResponse {
        repository_key: content.repository_key,
        from_generation_key: from.generation_key,
        from_generation_number: from.generation_number,
        from_ref_name: from.ref_name,
        from_commit_sha: from.commit_sha,
        to_generation_key: to.generation_key,
        to_generation_number: to.generation_number,
        to_ref_name: to.ref_name,
        to_commit_sha: to.commit_sha,
        index_status: content.index_status,
        checkpoint: content.checkpoint,
        from_file_count: from.file_count,
        from_excluded_file_count: from.excluded_file_count,
        from_total_bytes: from.total_bytes,
        to_file_count: to.file_count,
        to_excluded_file_count: to.excluded_file_count,
        to_total_bytes: to.total_bytes,
        changes: vec![
            GitRepositoryFileDiff {
                path: "src/added.rs".to_string(),
                change_kind: GitFileChangeKind::Added,
                before: None,
                after: Some(sample_diff_file("018f9f40-1111-7000-8000-0000000000c1")),
            },
            GitRepositoryFileDiff {
                path: "src/alpha.rs".to_string(),
                change_kind: GitFileChangeKind::Modified,
                before: Some(sample_diff_file("018f9f40-1111-7000-8000-0000000000c2")),
                after: Some(sample_diff_file("018f9f40-1111-7000-8000-0000000000c3")),
            },
            GitRepositoryFileDiff {
                path: "src/dropped.rs".to_string(),
                change_kind: GitFileChangeKind::Deleted,
                before: Some(sample_diff_file("018f9f40-1111-7000-8000-0000000000c4")),
                after: None,
            },
        ],
        pagination: CursorPagination::new(Some("3".to_string()), true),
    }
}
