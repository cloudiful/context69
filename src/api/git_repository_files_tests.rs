//! Focused tests for the active-generation manifest listing (issue #681 phase
//! 5A).
//!
//! The page is composed from already-persisted rows, so these tests build the
//! stored types in memory and never touch a database, a network, or a secret.
//! They pin how the route turns one bounded window into a page — provenance,
//! coverage, and the cursor continuation invariant — the exact non-content
//! response field set, the unchanged `GitRepositoryFile` projection, and the two
//! bounded error shapes an unknown or not-yet-indexed repository shares. The
//! request paging contract itself is tested where it is implemented, in
//! `git_repository_file_paging.rs`.

use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::{
    contracts::{
        CursorPageQuery, GroupKind, MembershipRole, Visibility,
        sources::{
            GitCommitCheckpoint, GitGenerationStatus, GitIndexProfile, GitIndexStatus,
            GitProviderKind, GitRefreshPolicy, GitRepositoryFile, GitVersionPolicy,
        },
    },
    db::{
        GitGroupOwnership, StoredGitGenerationFile, StoredGitRepositoryGeneration,
        StoredGitRepositorySource,
    },
    domain::GroupRecord,
    domain_errors::status_for_error,
};

use super::super::git_repository_file_paging::requested_page;
use super::super::group_access::group_access_error_response;
use super::{generation_not_ready, manifest_page, repository_not_found, require_manifest_read};

/// The group the listing is read from, carrying `current_role`.
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

fn generation_key() -> Uuid {
    Uuid::parse_str("018f9f40-2222-7000-8000-0000000000c2").expect("uuid")
}

fn repository_key() -> Uuid {
    Uuid::parse_str("018f9f3a-2c1b-7c4d-8e5f-6a7b8c9d0e1f").expect("uuid")
}

fn timestamp() -> DateTime<Utc> {
    "2026-10-02T00:00:00Z".parse().expect("timestamp")
}

fn source() -> StoredGitRepositorySource {
    StoredGitRepositorySource {
        group: GitGroupOwnership {
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

fn generation(file_count: i64) -> StoredGitRepositoryGeneration {
    StoredGitRepositoryGeneration {
        repository_key: repository_key(),
        generation_key: generation_key(),
        generation_number: 4,
        ref_name: "refs/heads/main".to_string(),
        commit_sha: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        index_profile: GitIndexProfile::Lexical,
        status: GitGenerationStatus::Ready,
        file_count,
        excluded_file_count: 3,
        total_bytes: 4096,
        error_code: None,
        started_at: timestamp(),
        completed_at: Some(timestamp()),
        created_at: timestamp(),
        updated_at: timestamp(),
    }
}

fn stored_file(path: &str) -> StoredGitGenerationFile {
    StoredGitGenerationFile {
        file_key: Uuid::parse_str("018f9f40-1111-7000-8000-0000000000c1").expect("uuid"),
        generation_key: generation_key(),
        repository_key: repository_key(),
        provider_blob_sha: "cccccccccccccccccccccccccccccccccccccccc".to_string(),
        path: path.to_string(),
        language: "rust".to_string(),
        byte_count: 512,
        line_count: 20,
        created_at: timestamp(),
    }
}

fn query(limit: u32, cursor: Option<&str>) -> CursorPageQuery {
    CursorPageQuery {
        limit,
        cursor: cursor.map(ToOwned::to_owned),
    }
}

#[test]
fn a_continued_page_reports_only_its_own_window() {
    let page = requested_page(&query(2, Some("4"))).expect("continuation");
    assert_eq!(page.offset, 4);

    let source = source();
    let generation = generation(10);
    let rows = vec![stored_file("src/e.rs"), stored_file("src/f.rs")];
    let body = manifest_page(&source, &generation, rows, &page, true);
    let paths: Vec<&str> = body.files.iter().map(|file| file.path.as_str()).collect();
    assert_eq!(paths, vec!["src/e.rs", "src/f.rs"]);
    assert_eq!(body.pagination.next_cursor.as_deref(), Some("6"));
    assert!(body.pagination.has_more);
    body.pagination
        .validate_continuation()
        .expect("has_more must carry its continuation");
}

#[test]
fn the_first_page_carries_no_cursor_when_nothing_follows() {
    let page = requested_page(&query(50, None)).expect("first page");
    let body = manifest_page(
        &source(),
        &generation(1),
        vec![stored_file("a.rs")],
        &page,
        false,
    );
    assert!(body.pagination.is_terminal());
    assert_eq!(body.pagination.next_cursor, None);
    assert!(!body.pagination.has_more);
    assert_eq!(body.files.len(), 1);
}

#[test]
fn the_page_reports_generation_provenance_checkpoint_and_coverage() {
    let page = requested_page(&query(50, None)).expect("first page");
    let source = source();
    let body = manifest_page(
        &source,
        &generation(120),
        vec![stored_file("src/main.rs")],
        &page,
        false,
    );

    assert_eq!(body.repository_key, source.repository_key);
    assert_eq!(body.generation_key, generation_key());
    assert_eq!(body.generation_number, 4);
    assert_eq!(body.ref_name, "refs/heads/main");
    assert_eq!(body.commit_sha, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    assert_eq!(body.index_status, GitIndexStatus::Stale);
    assert_eq!(
        body.checkpoint.indexed_commit_sha.as_deref(),
        Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
        "the page must show which commit is indexed, not only which one is served"
    );
    assert_eq!(
        body.checkpoint.target_commit_sha.as_deref(),
        Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
    );
    assert_eq!(body.checkpoint.indexed_at, Some(timestamp()));
    assert_eq!(body.file_count, 120);
    assert_eq!(body.excluded_file_count, 3);
    assert_eq!(body.total_bytes, 4096);

    let encoded = serde_json::to_value(&body).expect("page serializes");
    let object = encoded.as_object().expect("page object");
    let mut keys = object.keys().map(String::as_str).collect::<Vec<_>>();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "checkpoint",
            "commit_sha",
            "excluded_file_count",
            "file_count",
            "files",
            "generation_key",
            "generation_number",
            "index_status",
            "pagination",
            "ref_name",
            "repository_key",
            "total_bytes",
        ],
        "the manifest page exposes exactly the planned provenance, coverage, and pagination fields"
    );
    let serialized = serde_json::to_string(&body).expect("page serializes");
    for forbidden in [
        "internal/secret",
        "credential_secret_key",
        "provider_blob_sha",
        "connection_key",
        "canonical_url",
        "group_path",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "the manifest page must not project {forbidden}: {serialized}"
        );
    }
}

#[test]
fn the_file_projection_is_unchanged_and_carries_no_content() {
    let stored = stored_file("src/db/git_repositories/files.rs");
    let file = stored.to_contract();
    let encoded = serde_json::to_value(&file).expect("entry serializes");
    let mut keys = encoded
        .as_object()
        .expect("entry object")
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    keys.sort_unstable();
    assert_eq!(
        keys,
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
        "a manifest entry stays identity plus counters; bytes and chunks are read elsewhere"
    );
    assert_eq!(
        file,
        GitRepositoryFile {
            file_key: stored.file_key,
            generation_key: stored.generation_key,
            repository_key: stored.repository_key,
            path: "src/db/git_repositories/files.rs".to_string(),
            language: "rust".to_string(),
            byte_count: 512,
            line_count: 20,
            created_at: timestamp(),
        }
    );
    let serialized = serde_json::to_string(&file).expect("entry serializes");
    assert!(
        !serialized.contains("cccccccccccccccccccccccccccccccccccccccc"),
        "the provider blob id addresses stored bytes; it is not part of the wire entry: {serialized}"
    );
}

#[tokio::test]
async fn unknown_and_not_yet_indexed_repositories_share_bounded_error_shapes() {
    let not_found = repository_not_found();
    assert_eq!(not_found.status(), StatusCode::NOT_FOUND);
    let body = response_body(not_found).await;
    assert!(
        body.contains("unknown git repository"),
        "the unknown/foreign repository shape carries the bounded message: {body}"
    );

    // No active ready generation is a conflict, never an empty page: an empty
    // page would read like a repository whose manifest is genuinely empty.
    let not_ready = generation_not_ready();
    assert_eq!(not_ready.status(), StatusCode::CONFLICT);
    let body = response_body(not_ready).await;
    assert!(
        body.contains("no active index generation"),
        "the not-ready shape states the missing generation: {body}"
    );

    for detail in [
        "internal/secret",
        "commit",
        "github.com",
        "acme/team-platform",
    ] {
        assert!(
            !body.contains(detail),
            "the not-ready shape must not disclose repository detail {detail}: {body}"
        );
    }
}

#[test]
fn a_viewer_may_read_a_manifest_and_a_lower_or_absent_role_may_not() {
    // Read access starts at Viewer. The roles above it inherit the right
    // through the shared hierarchy rather than through a second rule here, so
    // the route states one floor and the hierarchy supplies the rest.
    for role in [
        MembershipRole::Viewer,
        MembershipRole::Maintainer,
        MembershipRole::Owner,
    ] {
        require_manifest_read(&group(Some(role)))
            .unwrap_or_else(|error| panic!("{role:?} may read a manifest: {error}"));
    }

    // An absent membership is the only state below Viewer, and it is refused.
    let error = require_manifest_read(&group(None))
        .expect_err("a caller with no membership must not read a manifest");
    assert_eq!(error.to_string(), "insufficient permissions for group");
    assert_eq!(
        status_for_error(&error),
        StatusCode::FORBIDDEN,
        "the refusal is the shared typed forbidden error, not a route-local one"
    );
}

#[tokio::test]
async fn a_refused_role_leaves_through_the_shared_group_access_shape() {
    let error = require_manifest_read(&group(None)).expect_err("no membership");
    let response = group_access_error_response(error);
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body = response_body(response).await;
    assert!(
        body.contains("insufficient permissions for group"),
        "the refusal is the shared message: {body}"
    );
    // Authorization is decided before the repository is read, so a refusal
    // cannot name a group, a repository, or a generation.
    for detail in [
        "acme/team-platform",
        "team-platform",
        "git-repositories",
        "commit",
    ] {
        assert!(
            !body.contains(detail),
            "the refusal must not disclose {detail}: {body}"
        );
    }
}

async fn response_body(response: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read bounded error body");
    String::from_utf8(bytes.to_vec()).expect("utf8 body")
}
