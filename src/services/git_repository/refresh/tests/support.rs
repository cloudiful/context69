//! Shared fixture helpers for the refresh tests.

use uuid::Uuid;

use crate::contracts::sources::GitIndexProfile;
use crate::db::{
    CreateTaskSubmissionRequest, GitGenerationCoverage, NewGitGenerationCodeVectors,
    NewGitRepositoryGeneration, NewGitRepositorySource,
};

use super::super::super::support::Fixture;

pub(super) async fn source_row(fixture: &Fixture) -> crate::db::StoredGitRepositorySource {
    fixture
        .db()
        .get_git_repository_source(fixture.group_id, fixture.repository_key)
        .await
        .expect("read source")
        .expect("the fixture owns the source")
}

pub(super) async fn due_contains(fixture: &Fixture) -> bool {
    fixture
        .db()
        .list_due_git_refresh_sources(10_000)
        .await
        .expect("list due sources")
        .iter()
        .any(|(_, key)| *key == fixture.repository_key)
}

pub(super) async fn pending_deletion_contains(fixture: &Fixture) -> bool {
    fixture
        .db()
        .list_pending_git_deletions(10_000)
        .await
        .expect("list pending deletions")
        .iter()
        .any(|(_, key)| *key == fixture.repository_key)
}

pub(super) async fn submit_git_index(
    fixture: &Fixture,
    user_id: i64,
    payloads: &[serde_json::Value],
) -> anyhow::Result<(Uuid, bool)> {
    let (task_id, reused, _) = fixture
        .db()
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id: Uuid::new_v4(),
            user_id,
            group_id: Some(fixture.group_id),
            kind: "git_index",
            group_path: Some("/git-refresh-test"),
            source_key: None,
            payloads,
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: "git-refresh-test",
        })
        .await?;
    Ok((task_id, reused))
}

pub(super) async fn start_and_activate(
    fixture: &Fixture,
    commit: &str,
    code: Option<NewGitGenerationCodeVectors>,
) -> Uuid {
    let generation = fixture
        .db()
        .start_git_repository_generation(
            fixture.group_id,
            fixture.repository_key,
            &NewGitRepositoryGeneration {
                ref_name: "refs/heads/main".to_string(),
                commit_sha: commit.to_string(),
                index_profile: GitIndexProfile::Hybrid,
            },
        )
        .await
        .expect("start generation");
    if let Some(code) = code {
        fixture
            .db()
            .record_git_generation_code_vectors(
                fixture.group_id,
                fixture.repository_key,
                generation.generation_key,
                &code,
            )
            .await
            .expect("record code vectors");
    }
    fixture
        .db()
        .complete_and_activate_git_repository_generation(
            fixture.group_id,
            fixture.repository_key,
            generation.generation_key,
            GitGenerationCoverage {
                file_count: 1,
                excluded_file_count: 0,
                total_bytes: 1,
            },
        )
        .await
        .expect("activate generation");
    generation.generation_key
}

pub(super) async fn seed_pending_cleanup_source(fixture: &Fixture, label: &str) -> Uuid {
    let nonce = Uuid::new_v4();
    let repository_key = fixture
        .db()
        .upsert_git_repository_source(
            fixture.group_id,
            &NewGitRepositorySource {
                connection_key: None,
                provider: crate::contracts::sources::GitProviderKind::GitHub,
                canonical_url: format!("https://github.com/octo/{label}-{nonce}"),
                owner: "octo".to_string(),
                name: label.to_string(),
                default_branch: "main".to_string(),
                target_ref: "refs/heads/main".to_string(),
                pinned_commit_sha: None,
                index_profile: GitIndexProfile::Hybrid,
                include_patterns: Vec::new(),
                exclude_patterns: Vec::new(),
                interval_seconds: None,
                webhook_enabled: false,
                enabled: true,
                retention_count: 1,
                created_by_user_id: None,
            },
        )
        .await
        .expect("register a pending-cleanup source")
        .repository_key;
    let generation = fixture
        .db()
        .start_git_repository_generation(
            fixture.group_id,
            repository_key,
            &NewGitRepositoryGeneration {
                ref_name: "refs/heads/main".to_string(),
                commit_sha: "a".repeat(40),
                index_profile: GitIndexProfile::Hybrid,
            },
        )
        .await
        .expect("start a generation")
        .generation_key;
    fixture
        .db()
        .record_git_generation_code_vectors(
            fixture.group_id,
            repository_key,
            generation,
            &NewGitGenerationCodeVectors {
                fingerprint: format!("fp-{label}-{nonce}"),
                endpoint_identity: format!("endpoint-{label}-{nonce}"),
                dimensions: 8,
                collection_name: format!("git_code_cleanup_{nonce}"),
                point_count: 1,
            },
        )
        .await
        .expect("record the code destination");
    fixture
        .db()
        .fail_git_repository_generation(
            fixture.group_id,
            repository_key,
            generation,
            "git_index_failed",
        )
        .await
        .expect("mark the generation failed");
    sqlx::query(
        "UPDATE context69.git_repository_generations \
         SET cleanup_pending = TRUE, \
             cleanup_error = 'git_code_sink_failed', \
             cleanup_attempts = 0, \
             cleanup_attempted_at = NULL, \
             cleanup_next_retry_at = NULL \
         WHERE generation_key = $1",
    )
    .bind(generation)
    .execute(fixture.db().pool())
    .await
    .expect("make the cleanup due immediately");
    repository_key
}

pub(super) async fn seed_pending_deletion_source(fixture: &Fixture, label: &str) -> Uuid {
    let nonce = Uuid::new_v4();
    let repository_key = fixture
        .db()
        .upsert_git_repository_source(
            fixture.group_id,
            &NewGitRepositorySource {
                connection_key: None,
                provider: crate::contracts::sources::GitProviderKind::GitHub,
                canonical_url: format!("https://github.com/octo/{label}-{nonce}"),
                owner: "octo".to_string(),
                name: label.to_string(),
                default_branch: "main".to_string(),
                target_ref: "refs/heads/main".to_string(),
                pinned_commit_sha: None,
                index_profile: GitIndexProfile::Hybrid,
                include_patterns: Vec::new(),
                exclude_patterns: Vec::new(),
                interval_seconds: None,
                webhook_enabled: false,
                enabled: true,
                retention_count: 1,
                created_by_user_id: None,
            },
        )
        .await
        .expect("register a deletion-pending source")
        .repository_key;
    let generation = fixture
        .db()
        .start_git_repository_generation(
            fixture.group_id,
            repository_key,
            &NewGitRepositoryGeneration {
                ref_name: "refs/heads/main".to_string(),
                commit_sha: "a".repeat(40),
                index_profile: GitIndexProfile::Hybrid,
            },
        )
        .await
        .expect("start a generation")
        .generation_key;
    fixture
        .db()
        .record_git_generation_code_vectors(
            fixture.group_id,
            repository_key,
            generation,
            &NewGitGenerationCodeVectors {
                fingerprint: format!("fp-del-{label}-{nonce}"),
                endpoint_identity: format!("endpoint-del-{label}-{nonce}"),
                dimensions: 8,
                collection_name: format!("git_code_deletion_{nonce}"),
                point_count: 1,
            },
        )
        .await
        .expect("record the code destination");
    fixture
        .db()
        .mark_git_repository_deletion_requested(fixture.group_id, repository_key)
        .await
        .expect("mark the source for deletion");
    repository_key
}
