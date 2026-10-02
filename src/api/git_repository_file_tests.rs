//! Focused tests for the exact-file metadata read (issue #681 phase 5B).
//!
//! The read composes already-persisted rows, so these tests build the stored
//! types in memory and never touch a database, a network, or a secret. They pin
//! the Viewer floor the route shares with the manifest listing, the path
//! validator that runs before any lookup, the exact non-content response field
//! set, the unchanged `GitRepositoryFile` projection, and the bounded error
//! shapes an unknown repository, an unsafe path, and an absent path share.

use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::{
    contracts::{
        GroupKind, MembershipRole, Visibility,
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
    services::git_repository::SafeTreePath,
};

use super::super::git_repository_files::{repository_not_found, require_manifest_read};
use super::{file_detail, found_entry, invalid_repository_path};

/// Longest path the acquisition path validator accepts; the query contract
/// declares the same bound so a generated client learns it from OpenAPI.
const PATH_MAX_CHARS: usize = 512;
/// Deepest path the acquisition path validator accepts.
const PATH_MAX_DEPTH: usize = 32;

fn generation_key() -> Uuid {
    Uuid::parse_str("018f9f40-2222-7000-8000-0000000000c2").expect("uuid")
}

fn repository_key() -> Uuid {
    Uuid::parse_str("018f9f3a-2c1b-7c4d-8e5f-6a7b8c9d0e1f").expect("uuid")
}

fn timestamp() -> DateTime<Utc> {
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

#[test]
fn the_exact_file_read_shares_the_manifest_viewer_floor() {
    // Both reads resolve their role through the same helper, so the detail read
    // cannot become a wider door than the manifest page.
    for role in [
        MembershipRole::Viewer,
        MembershipRole::Maintainer,
        MembershipRole::Owner,
    ] {
        require_manifest_read(&group(Some(role)))
            .unwrap_or_else(|error| panic!("{role:?} may read a file entry: {error}"));
    }
    assert!(
        require_manifest_read(&group(None)).is_err(),
        "a caller with no membership is refused before the path is even read"
    );
}

#[test]
fn only_a_storable_repository_path_passes_the_lookahead_validator() {
    // A stored entry's path always passes, so the value reaching the lookup is
    // exactly the value that was validated at storage time.
    for accepted in [
        "src/main.rs",
        "README.md",
        ".github/workflows/ci.yml",
        "crates/a-b/src/lib.rs",
        "docs/ünïcode/файл.md",
    ] {
        assert_eq!(
            SafeTreePath::parse(accepted)
                .expect("a stored path is storable")
                .as_str(),
            accepted
        );
    }

    for rejected in [
        "",
        "/etc/passwd",
        "src/../../etc/passwd",
        "src/./main.rs",
        "src//main.rs",
        "src\\main.rs",
        "src/main.rs\u{0}evil",
        "src/main.rs\n",
    ] {
        assert!(
            SafeTreePath::parse(rejected).is_err(),
            "path {rejected:?} must be refused before the lookup"
        );
    }

    // The declared bound is the acquisition bound, at and past the limit.
    let at_limit = "a".repeat(PATH_MAX_CHARS);
    assert!(SafeTreePath::parse(&at_limit).is_ok());
    assert!(SafeTreePath::parse(&"a".repeat(PATH_MAX_CHARS + 1)).is_err());
    let at_depth = (0..PATH_MAX_DEPTH)
        .map(|index| format!("s{index}"))
        .collect::<Vec<_>>()
        .join("/");
    assert!(SafeTreePath::parse(&at_depth).is_ok());
    let too_deep = (0..=PATH_MAX_DEPTH)
        .map(|index| format!("s{index}"))
        .collect::<Vec<_>>()
        .join("/");
    assert!(SafeTreePath::parse(&too_deep).is_err());
}

#[test]
fn the_response_carries_the_entry_and_the_serving_generation_only() {
    let source = source();
    let body = file_detail(
        &source,
        &generation(),
        stored_file("src/db/git_repositories/files.rs"),
    );

    assert_eq!(body.repository_key, repository_key());
    assert_eq!(body.generation_key, generation_key());
    assert_eq!(body.generation_number, 4);
    assert_eq!(body.ref_name, "refs/heads/main");
    assert_eq!(body.commit_sha, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    assert_eq!(body.index_status, GitIndexStatus::Stale);
    assert_eq!(
        body.checkpoint.indexed_commit_sha.as_deref(),
        Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
        "the detail states which commit is indexed, not only which one it serves"
    );
    assert_eq!(
        body.checkpoint.target_commit_sha.as_deref(),
        Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
    );
    assert_eq!(body.file_count, 120);
    assert_eq!(body.excluded_file_count, 3);
    assert_eq!(body.total_bytes, 4096);
    assert_eq!(body.file.path, "src/db/git_repositories/files.rs");
    assert_eq!(body.file.language, "rust");
    assert_eq!(body.file.byte_count, 512);
    assert_eq!(body.file.line_count, 20);

    let encoded = serde_json::to_value(&body).expect("detail serializes");
    let object = encoded.as_object().expect("detail object");
    let mut keys = object.keys().map(String::as_str).collect::<Vec<_>>();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "checkpoint",
            "commit_sha",
            "excluded_file_count",
            "file",
            "file_count",
            "generation_key",
            "generation_number",
            "index_status",
            "ref_name",
            "repository_key",
            "total_bytes",
        ],
        "the detail exposes exactly the entry plus generation provenance and coverage"
    );
    let mut entry_keys = object["file"]
        .as_object()
        .expect("entry object")
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    entry_keys.sort_unstable();
    assert_eq!(
        entry_keys,
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
        "the entry keeps the manifest page's exact field set"
    );

    let serialized = serde_json::to_string(&body).expect("detail serializes");
    for forbidden in [
        "cccccccccccccccccccccccccccccccccccccccc",
        "internal/secret",
        "provider_blob_sha",
        "\"text\"",
        "canonical_url",
        "connection_key",
        "group_path",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "the detail must not project {forbidden}: {serialized}"
        );
    }
}

#[test]
fn the_entry_projection_is_unchanged_from_the_manifest_page() {
    let stored = stored_file("src/main.rs");
    let file = stored.to_contract();
    assert_eq!(
        file,
        GitRepositoryFile {
            file_key: stored.file_key,
            generation_key: stored.generation_key,
            repository_key: stored.repository_key,
            path: "src/main.rs".to_string(),
            language: "rust".to_string(),
            byte_count: 512,
            line_count: 20,
            created_at: timestamp(),
        }
    );
    assert_eq!(
        serde_json::to_value(&file).expect("entry serializes"),
        serde_json::json!({
            "file_key": file.file_key,
            "generation_key": file.generation_key,
            "repository_key": file.repository_key,
            "path": "src/main.rs",
            "language": "rust",
            "byte_count": 512,
            "line_count": 20,
            "created_at": file.created_at,
        }),
        "a detail read returns the same entry a manifest page would return"
    );
}

#[tokio::test]
async fn the_lookup_result_mapping_answers_absent_paths_exactly_as_an_unknown_repository() {
    // The route maps the exact-file lookup result through `found_entry`, so this
    // exercises the arm the handler actually takes. A drift back to a
    // distinguishable "no such path" shape fails here.
    let unknown = repository_not_found();
    let unknown_status = unknown.status();
    let unknown_body = response_body(unknown).await;
    assert_eq!(unknown_status, StatusCode::NOT_FOUND);
    assert!(
        unknown_body.contains("unknown git repository"),
        "the shared shape carries the bounded message: {unknown_body}"
    );

    let absent = *found_entry(None).expect_err("an absent path is not an entry");
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

    // A stored row passes through untouched, so the mapping cannot swallow or
    // rewrite the entry the detail response is built from.
    let found =
        found_entry(Some(stored_file("src/main.rs"))).expect("a stored row is the entry itself");
    assert_eq!(found, stored_file("src/main.rs"));
}

#[tokio::test]
async fn an_unsafe_path_is_a_redacted_client_error() {
    // A rejected path is a malformed request, not a missing one: it stays a
    // distinct 400, and it never echoes the submitted value, the group, or any
    // stored path.
    let unsafe_path = invalid_repository_path();
    let unsafe_status = unsafe_path.status();
    assert_eq!(unsafe_status, StatusCode::BAD_REQUEST);
    let body = response_body(unsafe_path).await;
    assert!(
        body.contains("git_repository_path_invalid"),
        "the refusal names the rule, not the value: {body}"
    );
    for detail in [
        "passwd",
        "..",
        "\\\\",
        "src/",
        ".rs",
        "acme/team-platform",
        "github.com",
        "internal/secret",
    ] {
        assert!(
            !body.contains(detail),
            "an unsafe-path refusal must not echo {detail}: {body}"
        );
    }
    assert_ne!(
        unsafe_status,
        StatusCode::NOT_FOUND,
        "a rejected path is a client error, distinct from a missing one"
    );
}

async fn response_body(response: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read bounded error body");
    String::from_utf8(bytes.to_vec()).expect("utf8 body")
}
