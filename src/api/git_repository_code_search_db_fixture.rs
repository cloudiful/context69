//! In-memory test inputs for the bounded lexical code search (issue #681 phase
//! 5D).
//!
//! The builders produce the stored types the route composes from, so the route's
//! projection, bounds, and truncation are checked without a database.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::contracts::{
    GroupKind, MembershipRole, Visibility,
    sources::{
        GitCodeLexicalHit, GitCodeMatchKind, GitCodeSearchQuery, GitCommitCheckpoint,
        GitGenerationStatus, GitIndexProfile, GitIndexStatus, GitProviderKind, GitVersionPolicy,
    },
};
use crate::db::{GitGroupOwnership, StoredGitRepositoryGeneration, StoredGitRepositorySource};
use crate::domain::GroupRecord;

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
        include_patterns: Vec::new(),
        exclude_patterns: Vec::new(),
        interval_seconds: None,
        webhook_enabled: false,
        enabled: true,
        retention_count: 2,
        created_by_user_id: None,
        config_revision: 0,
        refresh_dirty: false,
        refresh_requested_at: None,
        last_refresh_attempt_at: None,
        force_rebuild_requested_at: None,
        deletion_requested_at: None,
        last_skip_summary: Default::default(),
        indexed_pipeline_version: 0,
        indexed_config_fingerprint: String::new(),
        index_profile: GitIndexProfile::Lexical,
        index_status: GitIndexStatus::Ready,
        last_error_code: None,
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
