//! Test inputs for the bounded lexical code search (issue #681 phase 5D).
//!
//! Two kinds live here, both `#[cfg(test)]`-only and both disposable. The
//! in-memory builders produce the stored types the route composes from, so the
//! route's projection, bounds, and truncation are checked without a database. The
//! database wrapper reuses the shared Git fixture, so no second repository
//! fixture exists: it supplies the chunk rows a lexical search needs and runs the
//! same group-scoped accessor the route calls, with the caller's visible groups
//! passed in as the route derives them from the resolved access scope.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::api::git_repository_files_db_fixture::Fixture;
use crate::contracts::{
    GroupKind, MembershipRole, Visibility,
    sources::{
        GitCodeLexicalHit, GitCodeMatchKind, GitCodeSearchQuery, GitCommitCheckpoint,
        GitGenerationStatus, GitIndexProfile, GitIndexStatus, GitProviderKind, GitRefreshPolicy,
        GitVersionPolicy,
    },
};
use crate::db::{
    GitCheckpointUpdate, GitGenerationCoverage, GitGroupOwnership, GitLexicalCodeSearch,
    NewGitGenerationChunk, StoredGitRepositoryGeneration, StoredGitRepositorySource,
};
use crate::domain::GroupRecord;

use crate::api::git_repository_files_db_fixture::{INDEXED_COMMIT, MANIFEST_ENTRIES};

/// The serving generation key every in-memory builder shares.
pub(super) fn generation_key() -> Uuid {
    Uuid::parse_str("018f9f40-2222-7000-8000-0000000000c2").expect("uuid")
}

/// The repository key every in-memory builder shares.
pub(super) fn repository_key() -> Uuid {
    Uuid::parse_str("018f9f3a-2c1b-7c4d-8e5f-6a7b8c9d0e1f").expect("uuid")
}

/// A fixed instant, so builders are comparable and never read the clock.
pub(super) fn timestamp() -> DateTime<Utc> {
    "2026-10-02T00:00:00Z".parse().expect("timestamp")
}

/// A private group with the membership role the route's Viewer floor checks.
pub(super) fn group(current_role: Option<MembershipRole>) -> GroupRecord {
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

/// A ready repository source that advertises [`generation_key`].
pub(super) fn source() -> StoredGitRepositorySource {
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
        index_status: GitIndexStatus::Ready,
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

/// The activated, ready generation [`source`] advertises.
pub(super) fn generation() -> StoredGitRepositoryGeneration {
    StoredGitRepositoryGeneration {
        repository_key: repository_key(),
        generation_key: generation_key(),
        generation_number: 7,
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

/// One stored lexical hit with its full provenance and verbatim text.
///
/// The text keeps its CRLF and trailing tab, so a projection that normalized the
/// stored bytes would be visible here.
pub(super) fn stored_hit(path: &str, score: f32, matched: GitCodeMatchKind) -> GitCodeLexicalHit {
    GitCodeLexicalHit {
        repository_key: repository_key(),
        generation_key: generation_key(),
        generation_number: 7,
        ref_name: "refs/heads/main".to_string(),
        commit_sha: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        visibility: Visibility::Private,
        file_key: Uuid::parse_str("018f9f40-1111-7000-8000-0000000000c1").expect("uuid"),
        path: path.to_string(),
        language: "rust".to_string(),
        chunk_key: Uuid::parse_str("018f9f40-3333-7000-8000-0000000000c3").expect("uuid"),
        chunk_index: 2,
        start_line: 41,
        end_line: 57,
        text: "fn window() {\r\n    1.0;\t\n}\n".to_string(),
        score,
        matched,
    }
}

/// The stored paths a search can match: two markdown files and one Rust file,
/// each carrying the same term, so one term exercises every filter and each
/// filter visibly narrows the same three matches.
const STORED_PATHS: [(&str, &str, &str); 3] = [
    ("README.md", "# manifest\n\nalpha marker\n", "markdown"),
    ("docs/guide.md", "guide text\n\nalpha marker\n", "markdown"),
    ("src/alpha.rs", "fn alpha() {}\n\nalpha marker\n", "rust"),
];

/// Indexes a generation whose files carry code, and activates it.
///
/// The shared fixture owns the group, repository, and manifest; this supplies the
/// chunk rows through the existing storage API and completes the same activation
/// the snapshot orchestration performs.
pub(super) async fn index_with_code(fixture: &Fixture) -> Uuid {
    let generation_key = fixture.start_building().await;
    fixture.store_manifest(generation_key).await;
    for (path, content, _) in STORED_PATHS {
        let file = fixture
            .file(generation_key, path)
            .await
            .expect("the indexed manifest holds the path");
        fixture
            .db
            .replace_git_file_chunks(
                fixture.group_id,
                fixture.repository_key,
                generation_key,
                file.file_key,
                &[NewGitGenerationChunk {
                    chunk_index: 0,
                    start_line: 1,
                    end_line: content.split_inclusive('\n').count() as i32,
                    text: content.to_string(),
                }],
            )
            .await
            .expect("store the chunk row");
    }
    fixture
        .db
        .complete_and_activate_git_repository_generation(
            fixture.group_id,
            fixture.repository_key,
            generation_key,
            GitGenerationCoverage {
                file_count: MANIFEST_ENTRIES as i64,
                excluded_file_count: 0,
                total_bytes: 4096,
            },
        )
        .await
        .expect("activate the generation");
    fixture
        .db
        .update_git_repository_checkpoint(
            fixture.group_id,
            fixture.repository_key,
            &GitCheckpointUpdate {
                target_commit_sha: Some(INDEXED_COMMIT.to_string()),
                indexed_commit_sha: Some(INDEXED_COMMIT.to_string()),
                index_status: GitIndexStatus::Ready,
            },
        )
        .await
        .expect("advance the source checkpoint")
        .expect("the repository is owned by this group");
    generation_key
}

/// A bounded search request, as a caller would send it.
pub(super) fn query(term: &str, limit: u8) -> GitCodeSearchQuery {
    GitCodeSearchQuery {
        query: term.to_string(),
        path_prefix: None,
        language: None,
        limit,
    }
}

/// The UTF-8 body of a bounded response, for comparing two shapes byte for byte.
pub(super) async fn response_body(response: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read the bounded body");
    String::from_utf8(bytes.to_vec()).expect("a UTF-8 body")
}

/// The classified language token stored for one of [`STORED_PATHS`].
pub(super) fn language_of(path: &str) -> &'static str {
    STORED_PATHS
        .iter()
        .find(|(stored, _, _)| *stored == path)
        .map(|(_, _, language)| *language)
        .expect("a stored path")
}

/// Runs the group-scoped lexical read the route runs, for one caller scope.
pub(super) async fn lexical(
    fixture: &Fixture,
    group_id: i64,
    visible_group_ids: Vec<i64>,
    query: &str,
    path_prefix: Option<&str>,
    language: Option<&str>,
    limit: i64,
) -> Vec<GitCodeLexicalHit> {
    fixture
        .db
        .lexical_search_git_generation_chunks(
            group_id,
            &GitLexicalCodeSearch {
                repository_key: fixture.repository_key,
                query: query.to_string(),
                path_prefix: path_prefix.map(str::to_string),
                language: language.map(str::to_string),
                visible_group_ids,
                limit,
            },
        )
        .await
        .expect("read the lexical hits")
}
