//! Focused tests for the bounded line-window content read (issue #681 phase 5C).
//!
//! The read is pure composition over already-persisted rows, so these tests
//! build stored sources, generations, entries, and chunks in memory and never
//! touch a database, a network, or a secret. They pin the Viewer floor the route
//! shares with the other Git reads, the bounded line window and continuation
//! cursor, the verbatim window trim, the byte cap and continuation invariant,
//! and the exact non-content field set of the response.

use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::{
    contracts::{
        GroupKind, MembershipRole, Visibility,
        sources::{
            GitCommitCheckpoint, GitGenerationStatus, GitIndexProfile, GitIndexStatus,
            GitProviderKind, GitRefreshPolicy, GitVersionPolicy,
        },
    },
    db::{
        StoredGitGenerationChunk, StoredGitGenerationFile, StoredGitRepositoryGeneration,
        StoredGitRepositorySource,
    },
    domain::GroupRecord,
};

use super::super::git_repository_files::{repository_not_found, require_manifest_read};
use super::{
    content_page, decode_cursor, file_content, found_content_entry, invalid_repository_path,
};

pub(super) fn generation_key() -> Uuid {
    Uuid::parse_str("018f9f40-2222-7000-8000-0000000000c2").expect("uuid")
}

fn repository_key() -> Uuid {
    Uuid::parse_str("018f9f3a-2c1b-7c4d-8e5f-6a7b8c9d0e1f").expect("uuid")
}

pub(super) fn file_key() -> Uuid {
    Uuid::parse_str("018f9f40-1111-7000-8000-0000000000c1").expect("uuid")
}

pub(super) fn timestamp() -> DateTime<Utc> {
    "2026-10-02T00:00:00Z".parse().expect("timestamp")
}

fn group(current_role: Option<MembershipRole>) -> GroupRecord {
    GroupRecord {
        id: 42,
        parent_group_id: None,
        group_path: "acme/team-platform".to_string(),
        parent_group_path: Some("acme".to_string()),
        group_key: "team-platform".to_string(),
        name: "Team Platform".to_string(),
        visibility: Visibility::Private,
        kind: GroupKind::Shared,
        owner_user_id: None,
        created_at: timestamp(),
        updated_at: timestamp(),
        current_role,
    }
}

fn source() -> StoredGitRepositorySource {
    StoredGitRepositorySource {
        group: crate::db::GitGroupOwnership {
            group_id: 42,
            group_key: "team-platform".to_string(),
            group_path: "acme/team-platform".to_string(),
            visibility: Visibility::Private,
        },
        repository_key: repository_key(),
        connection_key: Some("github-app-main".to_string()),
        provider: GitProviderKind::GitHub,
        canonical_url: "https://github.com/cloudiful/context69".to_string(),
        owner: "cloudiful".to_string(),
        name: "context69".to_string(),
        default_branch: "main".to_string(),
        version: GitVersionPolicy {
            ref_name: "refs/heads/main".to_string(),
            commit_sha: Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string()),
        },
        refresh: GitRefreshPolicy::Webhook,
        index_profile: GitIndexProfile::Lexical,
        index_status: GitIndexStatus::Stale,
        checkpoint: GitCommitCheckpoint {
            target_commit_sha: Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string()),
            indexed_commit_sha: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string()),
            indexed_at: Some(timestamp()),
            checkpoint_updated_at: Some(timestamp()),
        },
        active_generation_key: Some(generation_key()),
        created_at: timestamp(),
        updated_at: timestamp(),
    }
}

fn generation() -> StoredGitRepositoryGeneration {
    StoredGitRepositoryGeneration {
        repository_key: repository_key(),
        generation_key: generation_key(),
        generation_number: 4,
        ref_name: "refs/heads/main".to_string(),
        commit_sha: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        index_profile: GitIndexProfile::Lexical,
        status: GitGenerationStatus::Ready,
        file_count: 120,
        excluded_file_count: 3,
        total_bytes: 4096,
        error_code: None,
        started_at: timestamp(),
        completed_at: Some(timestamp()),
        created_at: timestamp(),
        updated_at: timestamp(),
    }
}

pub(super) fn file() -> StoredGitGenerationFile {
    StoredGitGenerationFile {
        file_key: file_key(),
        generation_key: generation_key(),
        repository_key: repository_key(),
        provider_blob_sha: "cccccccccccccccccccccccccccccccccccccccc".to_string(),
        path: "src/main.rs".to_string(),
        language: "rust".to_string(),
        byte_count: 512,
        line_count: 8,
        created_at: timestamp(),
    }
}

/// One stored chunk: its line range and its verbatim text. Shared with the
/// window tests, which assemble pages from the same rows.
pub(super) fn chunk(start_line: i32, end_line: i32, text: &str) -> StoredGitGenerationChunk {
    StoredGitGenerationChunk {
        chunk_key: Uuid::new_v4(),
        generation_key: generation_key(),
        file_key: file_key(),
        chunk_index: 0,
        start_line,
        end_line,
        text: text.to_string(),
        created_at: timestamp(),
    }
}

#[test]
fn the_content_read_shares_the_manifest_viewer_floor() {
    for role in [
        MembershipRole::Viewer,
        MembershipRole::Maintainer,
        MembershipRole::Owner,
    ] {
        require_manifest_read(&group(Some(role)))
            .unwrap_or_else(|error| panic!("{role:?} may read file content: {error}"));
    }
    assert!(
        require_manifest_read(&group(None)).is_err(),
        "a caller with no membership is refused before any content is read"
    );
}

#[test]
fn the_response_carries_the_text_the_bounds_and_the_generation_only() {
    let rows = vec![chunk(1, 2, "fn one() {}\nfn two() {}\n")];
    let page = content_page(&rows, 1, 2, 0);
    let body = file_content(&source(), &generation(), file(), 1, 2, page);

    assert_eq!(body.repository_key, repository_key());
    assert_eq!(body.generation_key, generation_key());
    assert_eq!(body.generation_number, 4);
    assert_eq!(body.commit_sha, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    assert_eq!(body.index_status, GitIndexStatus::Stale);
    assert_eq!(body.file_count, 120);
    assert_eq!(body.excluded_file_count, 3);
    assert_eq!(body.total_bytes, 4096);
    assert_eq!(body.start_line, 1);
    assert_eq!(body.end_line, 2);
    assert_eq!(body.text, "fn one() {}\nfn two() {}\n");
    assert_eq!(body.byte_count, 24);
    assert_eq!(body.file.path, "src/main.rs");

    let encoded = serde_json::to_value(&body).expect("content serializes");
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
        "the content response exposes the exact window text plus generation provenance"
    );
    // Text is the only content, and it is the window's own text: no blob bytes,
    // no provider blob id, no secret or connection state.
    let serialized = serde_json::to_string(&body).expect("content serializes");
    for forbidden in [
        "cccccccccccccccccccccccccccccccccccccccc",
        "internal/secret",
        "provider_blob_sha",
        "canonical_url",
        "connection_key",
        "group_path",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "the content response must not project {forbidden}: {serialized}"
        );
    }
}

#[tokio::test]
async fn refusals_stay_bounded_and_disclose_nothing() {
    let unsafe_path = invalid_repository_path();
    assert_eq!(unsafe_path.status(), StatusCode::BAD_REQUEST);
    let body = response_body(unsafe_path).await;
    assert!(body.contains("git_repository_path_invalid"), "{body}");
    for detail in ["passwd", "..", "acme/team-platform", "internal/secret"] {
        assert!(!body.contains(detail), "must not echo {detail}: {body}");
    }

    // A continuation this service never issued is a bounded client error.
    let refused = decode_cursor(Some("nope")).expect_err("malformed cursor");
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
    let body = response_body(*refused).await;
    assert!(body.contains("git_content_cursor_invalid"), "{body}");

    // The shared not-found shape the content read reuses reveals nothing about
    // a repository or a path.
    let unknown = repository_not_found();
    let unknown_body = response_body(unknown).await;
    assert!(
        unknown_body.contains("unknown git repository"),
        "{unknown_body}"
    );
    assert!(!unknown_body.contains("git_content"), "{unknown_body}");
}

#[tokio::test]
async fn the_lookup_result_mapping_answers_absent_paths_exactly_as_an_unknown_repository() {
    // The handler maps the exact-file lookup through `found_content_entry`, so
    // this exercises the arm the route actually takes: a drift back to a
    // distinguishable "no such path" shape fails here.
    let unknown = repository_not_found();
    let unknown_status = unknown.status();
    let unknown_body = response_body(unknown).await;
    assert_eq!(unknown_status, StatusCode::NOT_FOUND);
    assert!(
        unknown_body.contains("unknown git repository"),
        "the shared shape carries the bounded message: {unknown_body}"
    );

    let absent = *found_content_entry(None).expect_err("an absent path is not an entry");
    assert_eq!(
        absent.status(),
        unknown_status,
        "an absent path must not be distinguishable by status"
    );
    assert_eq!(
        response_body(absent).await,
        unknown_body,
        "an absent path must not be distinguishable by body either"
    );

    // A stored row passes through untouched, so the mapping can neither swallow
    // nor rewrite the entry the window and the response are built from.
    let found = found_content_entry(Some(file())).expect("a stored row is the entry itself");
    assert_eq!(found, file(), "the resolved entry is returned unchanged");
}

pub(super) async fn response_body(response: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read bounded error body");
    String::from_utf8(bytes.to_vec()).expect("utf8 body")
}
