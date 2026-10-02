//! Focused tests for the bounded lexical code search (issue #681 phase 5D).
//!
//! The search composes the resolved generation with the hits the storage layer
//! returned, so these tests build the stored types in memory and never touch a
//! database, a network, or a secret. They pin the shared Viewer floor, the
//! bounded `404`/`409` shapes, every wire-query bound, the `limit + 1`
//! truncation invariant, the field sets, and the absence of raw blob, provider,
//! and secret material.

use axum::http::StatusCode;

use crate::contracts::{
    MembershipRole, Visibility,
    sources::{
        GIT_CODE_SEARCH_LIMIT_DEFAULT, GitCodeMatchKind, GitCodeSearchQuery, GitIndexStatus,
    },
};

use super::super::errors::library_management_error_response;
use super::super::git_repository_files::{
    generation_not_ready, repository_not_found, require_manifest_read,
};
use super::fixture::{
    generation, generation_key, group, query, repository_key, response_body, source, stored_hit,
};
use super::{
    SearchFilters, found_generation, found_repository, is_language_token, search_response,
};

/// The commit both the repository and its serving generation are pinned to.
const COMMIT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

/// The refusal text one invalid request produces.
fn refusal(request: &GitCodeSearchQuery) -> String {
    SearchFilters::validated(request)
        .expect_err("an invalid request is refused")
        .to_string()
}

#[test]
fn the_code_search_shares_the_manifest_viewer_floor() {
    for role in [
        MembershipRole::Viewer,
        MembershipRole::Maintainer,
        MembershipRole::Owner,
    ] {
        require_manifest_read(&group(Some(role)))
            .unwrap_or_else(|error| panic!("{role:?} may search code: {error}"));
    }
    assert!(
        require_manifest_read(&group(None)).is_err(),
        "a caller with no membership is refused before any search runs"
    );
}

#[test]
fn a_term_is_bounded_and_never_rewritten() {
    // The term reaches the storage layer as it was matched, with only the
    // surrounding whitespace dropped, so a `%` or `_` stays a literal character
    // there rather than a pattern this read would have to escape again.
    for (sent, expected) in [
        ("SafeRef::parse", "SafeRef::parse"),
        ("100%", "100%"),
        ("a_b", "a_b"),
        ("  alpha  ", "alpha"),
    ] {
        let filters = SearchFilters::validated(&query(sent, 20))
            .unwrap_or_else(|error| panic!("{sent:?} is a bounded term: {error}"));
        assert_eq!(filters.query, expected, "{sent:?} is unmodified");
        assert_eq!(filters.fetch_limit(), 21, "one extra row is the probe");
    }
    for rejected in ["", "   ", "\t"] {
        let error = refusal(&query(rejected, 20));
        assert!(error.contains("git_code_search_query_invalid"), "{error}");
    }
    let error = refusal(&query(&"z".repeat(201), 20));
    assert!(error.contains("git_code_search_query_invalid"), "{error}");
    let at_bound = query(&"z".repeat(200), 20);
    assert!(
        SearchFilters::validated(&at_bound).is_ok(),
        "the bound is inclusive"
    );
}

#[test]
fn the_limit_is_bounded_and_defaults_documented() {
    for limit in [1, 20, 50] {
        let filters = SearchFilters::validated(&query("term", limit)).expect("a bounded limit");
        assert_eq!(filters.limit, i64::from(limit));
        assert_eq!(filters.fetch_limit(), i64::from(limit) + 1);
    }
    // The wire type defaults to the documented number, so a caller that names no
    // limit still gets a bounded page.
    assert_eq!(GIT_CODE_SEARCH_LIMIT_DEFAULT, 20);
    let defaulted: GitCodeSearchQuery =
        serde_json::from_value(serde_json::json!({ "query": "term" })).expect("decode a term");
    assert_eq!(defaulted.limit, GIT_CODE_SEARCH_LIMIT_DEFAULT);
    assert!(defaulted.path_prefix.is_none() && defaulted.language.is_none());
}

#[test]
fn a_path_prefix_is_a_safe_repository_relative_prefix() {
    // A directory prefix keeps its meaning without the empty trailing segment the
    // path rules refuse, and an absent, empty, or separator-only prefix narrows
    // nothing rather than matching nothing.
    for (sent, expected) in [
        (Some("src/api"), Some("src/api")),
        (Some("src/"), Some("src")),
        (Some("src/api/"), Some("src/api")),
        (Some(""), None),
        (Some("/"), None),
        (None, None),
    ] {
        let mut request = query("term", 20);
        request.path_prefix = sent.map(str::to_string);
        let filters = SearchFilters::validated(&request)
            .unwrap_or_else(|error| panic!("{sent:?} is a bounded prefix: {error}"));
        assert_eq!(filters.path_prefix.as_deref(), expected, "prefix {sent:?}");
    }
    for unsafe_prefix in ["/etc", "../etc", "src/../etc", "src/\0/term", "src/\n"] {
        let mut request = query("term", 20);
        request.path_prefix = Some(unsafe_prefix.to_string());
        assert!(
            refusal(&request).contains("git_code_search_path_prefix_invalid"),
            "prefix {unsafe_prefix:?} must be refused"
        );
    }
    let mut over_long = query("term", 20);
    over_long.path_prefix = Some("a".repeat(513));
    assert!(
        refusal(&over_long).contains("git_code_search_path_prefix_invalid"),
        "an over-long prefix must be refused"
    );
}

#[test]
fn a_language_token_is_the_classified_shape() {
    for token in [
        "rust",
        "python",
        "c++",
        "c#",
        "objective-c",
        "fsharp2",
        "mdx",
    ] {
        assert!(
            is_language_token(token),
            "{token} is a stored language token"
        );
    }
    for token in [
        "Rust",
        "rust ",
        "-rust",
        "rust!",
        "",
        "a".repeat(33).as_str(),
    ] {
        assert!(!is_language_token(token), "{token:?} is not a stored token");
    }
    let mut stored = query("term", 20);
    stored.language = Some("rust".to_string());
    assert_eq!(
        SearchFilters::validated(&stored)
            .expect("a stored language token")
            .language
            .as_deref(),
        Some("rust")
    );
    let mut invalid = query("term", 20);
    invalid.language = Some("Rust".to_string());
    assert!(refusal(&invalid).contains("git_code_search_language_invalid"));
}

#[test]
fn the_extra_row_is_the_only_truncation_evidence() {
    // One row per `count`, in the storage layer's score/path/chunk order, so the
    // only variable across the cases is how many rows came back and how many the
    // caller asked for.
    let hits = |count: usize| -> Vec<_> {
        (0..count)
            .map(|index| stored_hit(&format!("s{index}.rs"), 1.0, GitCodeMatchKind::ChunkPhrase))
            .collect()
    };
    for (count, limit, truncated) in [(1, 20, false), (2, 2, false), (3, 2, true), (0, 20, false)] {
        let response = search_response(&source(), &generation(), hits(count), limit);
        assert_eq!(
            response.truncated, truncated,
            "{count} rows for a limit of {limit}"
        );
        assert_eq!(
            response.hits.len(),
            count.min(limit as usize),
            "the probe row is evidence, never a returned hit"
        );
    }

    // A full but complete page keeps the storage layer's order, and an empty
    // result still carries the generation and its coverage, never an error.
    let more = search_response(&source(), &generation(), hits(3), 2);
    assert_eq!(
        more.hits.last().map(|hit| hit.path.as_str()),
        Some("s1.rs"),
        "the storage layer's order is preserved"
    );
    let empty = search_response(&source(), &generation(), hits(0), 20);
    assert_eq!(empty.generation_key, generation_key());
    assert_eq!(empty.file_count, 120);
}

#[test]
fn the_response_carries_provenance_coverage_and_verbatim_text() {
    let hit = stored_hit("src/api/mod.rs", 1.5, GitCodeMatchKind::ChunkTerms);
    let response = search_response(&source(), &generation(), vec![hit.clone()], 20);
    assert_eq!(response.repository_key, repository_key());
    assert_eq!(response.generation_key, generation_key());
    assert_eq!(response.generation_number, 7);
    assert_eq!(response.ref_name, "refs/heads/main");
    assert_eq!(response.commit_sha, COMMIT);
    assert_eq!(response.index_status, GitIndexStatus::Ready);
    assert_eq!(
        response.checkpoint.indexed_commit_sha.as_deref(),
        Some(COMMIT)
    );
    assert_eq!(
        (response.file_count, response.excluded_file_count),
        (120, 3)
    );
    assert_eq!(response.total_bytes, 4096);

    let projected = &response.hits[0];
    assert_eq!(
        (projected.path.as_str(), projected.language.as_str()),
        ("src/api/mod.rs", "rust")
    );
    assert_eq!((projected.start_line, projected.end_line), (41, 57));
    assert_eq!((projected.chunk_index, projected.score), (2, 1.5));
    assert_eq!(projected.matched, GitCodeMatchKind::ChunkTerms);
    assert_eq!(projected.visibility, Visibility::Private);
    assert_eq!(projected.generation_number, 7);
    assert_eq!(
        projected.text, "fn window() {\r\n    1.0;\t\n}\n",
        "the stored chunk text is verbatim, CRLF and trailing tab included"
    );

    let encoded = serde_json::to_value(&response).expect("search serializes");
    let mut keys: Vec<&str> = encoded
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys.join(" "),
        "checkpoint commit_sha excluded_file_count file_count generation_key \
         generation_number hits index_status ref_name repository_key total_bytes truncated",
        "the search response exposes the generation, its coverage, the bounded hits, \
         and the truncation flag — nothing else"
    );
    let stored = serde_json::to_value(hit).expect("the stored hit serializes");
    let mut projected_keys: Vec<&str> = encoded["hits"][0]
        .as_object()
        .expect("hit object")
        .keys()
        .map(String::as_str)
        .collect();
    let mut stored_keys: Vec<&str> = stored
        .as_object()
        .expect("stored hit object")
        .keys()
        .map(String::as_str)
        .collect();
    projected_keys.sort_unstable();
    stored_keys.sort_unstable();
    assert_eq!(
        projected_keys, stored_keys,
        "a hit repeats the stored provenance field for field, verbatim text included"
    );
    // The key sets above are exact, and this reaches inside nested values too.
    let serialized = encoded.to_string();
    for forbidden in ["pagination", "next_cursor", "blob", "secret", "connection"] {
        assert!(
            !serialized.contains(forbidden),
            "the search must not expose {forbidden}: {serialized}"
        );
    }
}

/// The bounded refusal one invalid request would return: its status and body.
async fn refusal_response(request: &GitCodeSearchQuery) -> (u16, String) {
    let error = SearchFilters::validated(request).expect_err("an invalid request is refused");
    let response = library_management_error_response(error);
    let status = response.status().as_u16();
    (status, response_body(response).await)
}

#[tokio::test]
async fn a_refused_search_is_a_bounded_400_that_echoes_the_rule_only() {
    const SUBMITTED: &str = "secret-term";
    let over_long = "z".repeat(201);
    let mut bad_language = query(SUBMITTED, 20);
    bad_language.language = Some("Rust".to_string());
    for (request, rule) in [
        (query(&over_long, 20), "git_code_search_query_invalid"),
        (query(SUBMITTED, 0), "git_code_search_limit_out_of_bounds"),
        (query(SUBMITTED, 51), "git_code_search_limit_out_of_bounds"),
        // The language rule is a shape, so a refused token is not echoed either.
        (bad_language, "git_code_search_language_invalid"),
    ] {
        let (status, body) = refusal_response(&request).await;
        assert_eq!(
            status, 400,
            "an invalid search is a client error, not a server or not-found one"
        );
        assert!(body.contains(rule), "the refusal names the rule: {body}");
        assert!(
            !body.contains(SUBMITTED) && !body.contains(over_long.as_str()),
            "the refusal must not echo the submitted value: {body}"
        );
    }
}

#[tokio::test]
async fn an_unresolvable_repository_answers_exactly_as_an_unknown_repository() {
    // The handler maps the group-scoped lookup through `found_repository`, so
    // this exercises the arm the route actually takes. A drift back to a
    // distinguishable "no such repository for this group" shape fails here.
    let unknown = repository_not_found();
    let unknown_status = unknown.status();
    let unknown_body = response_body(unknown).await;
    assert_eq!(unknown_status, StatusCode::NOT_FOUND);
    assert!(
        unknown_body.contains("unknown git repository"),
        "the shared shape carries the bounded message: {unknown_body}"
    );

    // A foreign repository and a key that names nothing are the same `None`.
    let absent = *found_repository(Ok(None)).expect_err("no source is not a source");
    assert_eq!(
        absent.status(),
        unknown_status,
        "the status must be shared too"
    );
    assert_eq!(
        response_body(absent).await,
        unknown_body,
        "an unresolvable repository must not be distinguishable by body either"
    );
    // The bounded message is the whole body: no repository key, provider, or
    // owner detail rides along with it.
    assert!(!unknown_body.contains(&repository_key().to_string()));
    for detail in ["github", "cloudiful", "team-platform"] {
        assert!(!unknown_body.contains(detail), "echoes {detail}");
    }

    // The stored source passes through untouched, so the mapping can neither
    // swallow nor rewrite what the generation lookup and the response use.
    let stored = found_repository(Ok(Some(source()))).expect("a stored source is a source");
    assert_eq!(stored.repository_key, repository_key());
}

#[tokio::test]
async fn a_repository_without_a_serving_generation_is_a_bounded_conflict() {
    // The handler maps the serving-generation lookup through `found_generation`,
    // so an empty hit list can never stand in for "nothing is indexed yet".
    let not_ready = generation_not_ready();
    let not_ready_status = not_ready.status();
    let not_ready_body = response_body(not_ready).await;
    assert_eq!(not_ready_status, StatusCode::CONFLICT);
    assert!(
        not_ready_body.contains("no active index generation"),
        "the shared shape carries the bounded message: {not_ready_body}"
    );

    let absent = *found_generation(Ok(None)).expect_err("no generation is not one");
    assert_eq!(
        absent.status(),
        not_ready_status,
        "a repository with no serving generation is a conflict, not a client error"
    );
    assert_eq!(
        response_body(absent).await,
        not_ready_body,
        "the 409 must be the shared one, byte for byte"
    );
    let echoed_generation_key = generation_key().to_string();
    assert!(
        !not_ready_body.contains(&echoed_generation_key)
            && !not_ready_body.contains("index_status"),
        "the 409 must not echo generation or index detail: {not_ready_body}"
    );

    // The stored generation passes through untouched, so the response is built
    // from the generation the repository actually advertises.
    let stored = found_generation(Ok(Some(generation()))).expect("a stored generation is one");
    assert_eq!(stored.generation_key, generation_key());
}
