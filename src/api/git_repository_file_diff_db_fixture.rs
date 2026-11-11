//! In-memory test inputs for the metadata-only generation comparison (issue #681
//! phase 5E).
//!
//! The builders produce the stored types the page is composed from, so the
//! projection, the change kinds, the ordering, and the continuation are checked
//! without a database.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::contracts::{
    GroupKind, MembershipRole, Visibility,
    sources::{
        GitCommitCheckpoint, GitFileChangeKind, GitGenerationStatus, GitIndexProfile,
        GitIndexStatus, GitProviderKind, GitVersionPolicy,
    },
};
use crate::db::{
    GitFileDiffSide, StoredGitFileDiff, StoredGitRepositoryGeneration, StoredGitRepositorySource,
};
use crate::domain::GroupRecord;

/// The compared-from generation every in-memory builder shares.
pub(super) const FROM_COMMIT: &str = "1111111111111111111111111111111111111111";
/// The compared-to generation every in-memory builder shares.
pub(super) const TO_COMMIT: &str = "2222222222222222222222222222222222222222";
/// The commit the source says it has indexed, which is the compared-from one.
pub(super) const INDEXED_COMMIT: &str = FROM_COMMIT;
/// The target commit the source is still waiting for.
pub(super) const TARGET_COMMIT: &str = "3333333333333333333333333333333333333333";

/// The compared-from generation key every in-memory builder shares.
pub(super) fn from_key() -> Uuid {
    Uuid::parse_str("018f9f40-2222-7000-8000-0000000000c2").expect("uuid")
}

/// The compared-to generation key every in-memory builder shares.
pub(super) fn to_key() -> Uuid {
    Uuid::parse_str("018f9f40-3333-7000-8000-0000000000c3").expect("uuid")
}

/// The repository key every in-memory builder shares.
pub(super) fn repository_key() -> Uuid {
    Uuid::parse_str("018f9f3a-2c1b-7c4d-8e5f-6a7b8c9d0e1f").expect("uuid")
}

/// A file key derived from a position, so before/after entries are distinct.
pub(super) fn file_key(ordinal: u8) -> Uuid {
    Uuid::parse_str(&format!("018f9f40-1111-7000-8000-00000000000{ordinal}")).expect("uuid")
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

/// A ready repository source whose checkpoint names the compared-from commit,
/// so the default starting point of a comparison resolves to a stored
/// generation.
pub(super) fn source() -> StoredGitRepositorySource {
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
            commit_sha: Some(TARGET_COMMIT.to_string()),
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
        index_status: GitIndexStatus::Stale,
        last_error_code: None,
        checkpoint: GitCommitCheckpoint {
            target_commit_sha: Some(TARGET_COMMIT.to_string()),
            indexed_commit_sha: Some(INDEXED_COMMIT.to_string()),
            indexed_at: Some(timestamp()),
            checkpoint_updated_at: Some(timestamp()),
        },
        active_generation_key: Some(to_key()),
        created_at: timestamp(),
        updated_at: timestamp(),
    }
}

/// The older of the two compared generations.
pub(super) fn from_generation() -> StoredGitRepositoryGeneration {
    generation(7, from_key(), FROM_COMMIT, 120, 3, 4096)
}

/// The newer of the two compared generations.
pub(super) fn to_generation() -> StoredGitRepositoryGeneration {
    generation(8, to_key(), TO_COMMIT, 122, 3, 4300)
}

/// One generation, with the coverage envelope the response reports.
pub(super) fn generation(
    number: i64,
    generation_key: Uuid,
    commit_sha: &str,
    file_count: i64,
    excluded_file_count: i64,
    total_bytes: i64,
) -> StoredGitRepositoryGeneration {
    StoredGitRepositoryGeneration {
        repository_key: repository_key(),
        generation_key,
        generation_number: number,
        ref_name: "refs/heads/main".to_string(),
        commit_sha: commit_sha.to_string(),
        index_profile: GitIndexProfile::Lexical,
        status: GitGenerationStatus::Ready,
        file_count,
        excluded_file_count,
        total_bytes,
        error_code: None,
        started_at: timestamp(),
        completed_at: Some(timestamp()),
        created_at: timestamp(),
        updated_at: timestamp(),
    }
}

/// One stored change: the metadata a caller gets for one changed path.
pub(super) fn stored_diff(
    path: &str,
    change_kind: GitFileChangeKind,
    before: Option<(u8, &str, i64, i64)>,
    after: Option<(u8, &str, i64, i64)>,
) -> StoredGitFileDiff {
    StoredGitFileDiff {
        path: path.to_string(),
        change_kind,
        before: before.map(diff_side),
        after: after.map(diff_side),
    }
}

fn diff_side((ordinal, language, byte_count, line_count): (u8, &str, i64, i64)) -> GitFileDiffSide {
    GitFileDiffSide {
        file_key: file_key(ordinal),
        language: language.to_string(),
        byte_count,
        line_count,
    }
}

/// One change of each kind, in path order, as the comparison would return them.
pub(super) fn changed() -> Vec<StoredGitFileDiff> {
    vec![
        stored_diff(
            "src/added.rs",
            GitFileChangeKind::Added,
            None,
            Some((1, "rust", 24, 2)),
        ),
        stored_diff(
            "src/alpha.rs",
            GitFileChangeKind::Modified,
            Some((2, "rust", 20, 2)),
            Some((3, "rust", 40, 4)),
        ),
        stored_diff(
            "src/dropped.rs",
            GitFileChangeKind::Deleted,
            Some((4, "rust", 24, 2)),
            None,
        ),
    ]
}

/// A bounded page request, as the shared paging contract reads it.
pub(super) fn page_query(limit: u32, cursor: Option<&str>) -> crate::contracts::CursorPageQuery {
    crate::contracts::CursorPageQuery {
        limit,
        cursor: cursor.map(str::to_string),
    }
}

/// One composed page over the in-memory inputs, windowed the way the read is:
/// the token's offset, at most the limit, and the extra row as the only evidence
/// that another page exists.
pub(super) fn page(
    changes: &[StoredGitFileDiff],
    limit: u32,
    cursor: Option<&str>,
) -> crate::contracts::sources::GitRepositoryFileDiffResponse {
    let request =
        super::super::git_repository_file_paging::requested_page(&page_query(limit, cursor))
            .expect("a bounded page");
    let offset = request.offset as usize;
    let has_more = changes.len() - offset > request.limit() as usize;
    let kept = changes
        .iter()
        .skip(offset)
        .take(request.limit() as usize)
        .cloned()
        .collect::<Vec<_>>();
    super::diff_page(
        &source(),
        &from_generation(),
        &to_generation(),
        kept,
        &request,
        has_more,
    )
}

/// The UTF-8 body of a bounded response, for comparing two shapes byte for byte.
pub(super) async fn response_body(response: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read the bounded body");
    String::from_utf8(bytes.to_vec()).expect("a UTF-8 body")
}
