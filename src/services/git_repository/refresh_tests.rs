//! Refresh convergence, single-flight admission, and retention/disposal
//! behavior (issue #28 phase P4).
//!
//! Every case runs against the migrated scratch database named by
//! `CONTEXT69_TEST_DATABASE_URL`. They exercise the durable demand marker
//! shared by the scheduler, manual sync, and webhook dispatch, the per-source
//! admission fence, and vector cleanup failures that must retain ownership for
//! a retry. Groups created here are deleted with their cascade, and the one
//! task user is removed explicitly.

use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

use crate::contracts::sources::{GitGenerationStatus, GitIndexProfile, GitIndexStatus};
use crate::db::{
    CreateTaskSubmissionRequest, GitGenerationCoverage, GitIndexLifetimeGuard,
    GitRepositorySettingsUpdate, NewGitGenerationCodeVectors, NewGitRepositoryGeneration,
    NewGitRepositorySource,
};
use crate::domain_errors::DomainError;

use super::code_vectors::code_endpoint_identity;
use super::refresh::{
    cleanup_deleted_source, prune_generations, remove_generation_vectors,
    run_pending_generation_cleanup,
};
use super::support::Fixture;

async fn source_row(fixture: &Fixture) -> crate::db::StoredGitRepositorySource {
    fixture
        .db()
        .get_git_repository_source(fixture.group_id, fixture.repository_key)
        .await
        .expect("read source")
        .expect("the fixture owns the source")
}

async fn due_contains(fixture: &Fixture) -> bool {
    fixture
        .db()
        .list_due_git_refresh_sources(10_000)
        .await
        .expect("list due sources")
        .iter()
        .any(|(_, key)| *key == fixture.repository_key)
}

async fn pending_deletion_contains(fixture: &Fixture) -> bool {
    fixture
        .db()
        .list_pending_git_deletions(10_000)
        .await
        .expect("list pending deletions")
        .iter()
        .any(|(_, key)| *key == fixture.repository_key)
}

async fn seed_user(db: &crate::db::Database) -> i64 {
    sqlx::query(
        "INSERT INTO context69.users (login_name, display_name, password_hash) \
         VALUES ($1, 'Git Refresh Test', 'x') RETURNING id",
    )
    .bind(format!("git-refresh-{}", Uuid::new_v4()))
    .fetch_one(db.pool())
    .await
    .expect("seed user")
    .get("id")
}

async fn submit_git_index(
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

async fn start_and_activate(
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

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_demand_recorded_during_a_run_survives_its_completion() {
    let fixture = Fixture::setup().await;
    let user_id = seed_user(fixture.db()).await;
    let payloads = [json!({ "repository_key": fixture.repository_key.to_string() })];

    let (first_task, first_reused) = submit_git_index(&fixture, user_id, &payloads)
        .await
        .expect("the first submission admits a task");
    assert!(!first_reused);

    fixture
        .db()
        .request_git_refresh(fixture.group_id, fixture.repository_key)
        .await
        .expect("record the in-flight demand");
    let observed = source_row(&fixture).await.refresh_requested_at;
    let observed = observed.expect("the demand carries its consume token");

    let (second_task, second_reused) = submit_git_index(&fixture, user_id, &payloads)
        .await
        .expect("a request during the run is admitted idempotently");
    assert!(second_reused, "one repository has at most one active task");
    assert_eq!(first_task, second_task);

    let after = source_row(&fixture).await;
    assert!(after.refresh_dirty, "the newer demand stays pending");
    assert!(
        after.refresh_requested_at > Some(observed),
        "the follow-up carries a later consume token"
    );

    assert!(
        !fixture
            .db()
            .consume_git_refresh_request(fixture.group_id, fixture.repository_key, observed)
            .await
            .expect("consume the older demand"),
        "a run that observed the older demand must not clear the newer one"
    );
    assert!(source_row(&fixture).await.refresh_dirty);
    assert!(
        fixture
            .db()
            .consume_git_refresh_request(
                fixture.group_id,
                fixture.repository_key,
                after.refresh_requested_at.expect("newer token"),
            )
            .await
            .expect("consume the observed demand"),
        "the run that observed the newest demand clears it"
    );
    assert!(!source_row(&fixture).await.refresh_dirty);

    let task_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM context69.tasks \
         WHERE group_id = $1 AND kind = 'git_index'",
    )
    .bind(fixture.group_id)
    .fetch_one(fixture.db().pool())
    .await
    .expect("count tasks");
    assert_eq!(task_count, 1, "no competing writer was created");

    let db = fixture.db().clone();
    fixture.cleanup().await;
    sqlx::query("DELETE FROM context69.users WHERE id = $1")
        .bind(user_id)
        .execute(db.pool())
        .await
        .expect("clean up the task user");
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn due_selection_honors_demand_interval_backoff_and_disablement() {
    let fixture = Fixture::setup().await;
    assert!(
        !due_contains(&fixture).await,
        "a manual source with no demand is never due"
    );

    fixture
        .db()
        .request_git_refresh(fixture.group_id, fixture.repository_key)
        .await
        .expect("record a demand");
    assert!(due_contains(&fixture).await, "a pending demand is due");

    fixture
        .db()
        .mark_git_refresh_attempt(fixture.group_id, fixture.repository_key)
        .await
        .expect("mark the attempt");
    assert!(
        !due_contains(&fixture).await,
        "the 60-second window suppresses a retry immediately after an attempt"
    );

    sqlx::query(
        "UPDATE context69.git_repository_sources \
         SET last_refresh_attempt_at = now() - interval '61 seconds' \
         WHERE repository_key = $1",
    )
    .bind(fixture.repository_key)
    .execute(fixture.db().pool())
    .await
    .expect("age the attempt");
    assert!(
        due_contains(&fixture).await,
        "an old attempt lets the pending demand retry"
    );

    fixture
        .db()
        .consume_git_refresh_request(
            fixture.group_id,
            fixture.repository_key,
            source_row(&fixture)
                .await
                .refresh_requested_at
                .expect("demand token"),
        )
        .await
        .expect("consume the demand");
    assert!(
        !due_contains(&fixture).await,
        "with no demand and no interval the timer is off"
    );

    fixture
        .db()
        .update_git_repository_source_settings(
            fixture.group_id,
            fixture.repository_key,
            &GitRepositorySettingsUpdate {
                target_ref: None,
                pinned_commit_action: "keep",
                pinned_commit_value: None,
                index_profile: None,
                include_patterns: &[],
                exclude_patterns: &[],
                interval_seconds: Some(300),
                webhook_enabled: false,
                enabled: true,
                retention_count: 2,
            },
        )
        .await
        .expect("enable the interval")
        .expect("the fixture owns the source");
    assert!(
        !due_contains(&fixture).await,
        "a freshly indexed source waits for its interval"
    );
    sqlx::query(
        "UPDATE context69.git_repository_sources \
         SET last_indexed_at = now() - interval '301 seconds', \
             created_at = now() - interval '301 seconds', \
             last_refresh_attempt_at = NULL \
         WHERE repository_key = $1",
    )
    .bind(fixture.repository_key)
    .execute(fixture.db().pool())
    .await
    .expect("age the interval");
    assert!(
        due_contains(&fixture).await,
        "an elapsed interval admits the source without a demand"
    );

    fixture
        .db()
        .mark_git_repository_deletion_requested(fixture.group_id, fixture.repository_key)
        .await
        .expect("mark deletion");
    assert!(
        !due_contains(&fixture).await,
        "a deletion-pending source is never scheduled"
    );

    fixture.cleanup().await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn settings_edits_invalidate_the_snapshot_and_clear_the_failure_code() {
    let fixture = Fixture::setup().await;
    let config_revision = source_row(&fixture).await.config_revision;
    fixture
        .db()
        .mark_git_repository_failure(
            fixture.group_id,
            fixture.repository_key,
            config_revision,
            "git_index_upstream_error",
        )
        .await
        .expect("record a bounded failure");
    let failed = source_row(&fixture).await;
    assert_eq!(failed.index_status, GitIndexStatus::Failed);
    assert_eq!(
        failed.last_error_code.as_deref(),
        Some("git_index_upstream_error")
    );

    let edited = fixture
        .db()
        .update_git_repository_source_settings(
            fixture.group_id,
            fixture.repository_key,
            &GitRepositorySettingsUpdate {
                target_ref: Some("refs/tags/v1.0.0"),
                pinned_commit_action: "clear",
                pinned_commit_value: None,
                index_profile: Some("hybrid"),
                include_patterns: &[],
                exclude_patterns: &[],
                interval_seconds: None,
                webhook_enabled: false,
                enabled: true,
                retention_count: 2,
            },
        )
        .await
        .expect("apply settings")
        .expect("the fixture owns the source");
    assert_eq!(edited.version.ref_name, "refs/tags/v1.0.0");
    assert_eq!(edited.version.commit_sha, None);
    assert_eq!(edited.index_profile, GitIndexProfile::Hybrid);
    assert_eq!(edited.index_status, GitIndexStatus::Stale);
    assert_eq!(edited.last_error_code, None);

    let config_revision = source_row(&fixture).await.config_revision;
    fixture
        .db()
        .mark_git_repository_failure(
            fixture.group_id,
            fixture.repository_key,
            config_revision,
            "git_index_failed",
        )
        .await
        .expect("record another failure");
    let disabled = fixture
        .db()
        .update_git_repository_source_settings(
            fixture.group_id,
            fixture.repository_key,
            &GitRepositorySettingsUpdate {
                target_ref: None,
                pinned_commit_action: "keep",
                pinned_commit_value: None,
                index_profile: None,
                include_patterns: &[],
                exclude_patterns: &[],
                interval_seconds: None,
                webhook_enabled: false,
                enabled: false,
                retention_count: 2,
            },
        )
        .await
        .expect("disable the source")
        .expect("the fixture owns the source");
    assert_eq!(disabled.index_status, GitIndexStatus::Disabled);
    assert_eq!(disabled.last_error_code, None);

    fixture.cleanup().await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn retention_count_is_the_total_including_the_active_snapshot() {
    let fixture = Fixture::setup().await;
    let first = start_and_activate(&fixture, &"1".repeat(40), None).await;
    let second = start_and_activate(&fixture, &"2".repeat(40), None).await;
    let third = start_and_activate(&fixture, &"3".repeat(40), None).await;

    let failures = prune_generations(
        fixture.db(),
        fixture.group_id,
        fixture.repository_key,
        2,
        None,
    )
    .await
    .expect("prune with a total of two");
    assert_eq!(failures, 0, "lexical generations need no vector cleanup");
    let generations = fixture
        .db()
        .list_git_repository_generations(fixture.group_id, fixture.repository_key)
        .await
        .expect("read generations");
    let keys: Vec<Uuid> = generations
        .iter()
        .map(|generation| generation.generation_key)
        .collect();
    assert_eq!(
        keys.len(),
        2,
        "a total of two retains the active snapshot and one previous"
    );
    assert!(keys.contains(&second) && keys.contains(&third));
    assert!(!keys.contains(&first), "the oldest snapshot is removed");

    let failures = prune_generations(
        fixture.db(),
        fixture.group_id,
        fixture.repository_key,
        1,
        None,
    )
    .await
    .expect("prune with a total of one");
    assert_eq!(failures, 0);
    let generations = fixture
        .db()
        .list_git_repository_generations(fixture.group_id, fixture.repository_key)
        .await
        .expect("read generations");
    assert_eq!(generations.len(), 1, "a total of one keeps the active only");
    assert_eq!(generations[0].generation_key, third);
    let active = fixture
        .db()
        .get_git_active_generation(fixture.group_id, fixture.repository_key)
        .await
        .expect("read active generation")
        .expect("the newest generation stays active");
    assert_eq!(active.generation_key, third);

    let building = fixture
        .db()
        .start_git_repository_generation(
            fixture.group_id,
            fixture.repository_key,
            &NewGitRepositoryGeneration {
                ref_name: "refs/heads/main".to_string(),
                commit_sha: "4".repeat(40),
                index_profile: GitIndexProfile::Hybrid,
            },
        )
        .await
        .expect("start an in-use building generation")
        .generation_key;
    let failures = prune_generations(
        fixture.db(),
        fixture.group_id,
        fixture.repository_key,
        1,
        None,
    )
    .await
    .expect("prune around an in-use generation");
    assert_eq!(failures, 0);
    let generations = fixture
        .db()
        .list_git_repository_generations(fixture.group_id, fixture.repository_key)
        .await
        .expect("read generations");
    assert!(
        generations
            .iter()
            .any(|generation| generation.generation_key == building),
        "a building generation younger than one hour is never a candidate"
    );

    fixture.cleanup().await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn independent_cleanup_retry_finishes_for_a_disabled_manual_source() {
    let fixture = Fixture::setup().await;
    let metadata = NewGitGenerationCodeVectors {
        fingerprint: "git-code-fingerprint".to_string(),
        endpoint_identity: "git-code-endpoint-identity".to_string(),
        dimensions: 8,
        collection_name: "git_code_retention_test".to_string(),
        point_count: 1,
    };
    let retired = start_and_activate(&fixture, &"b".repeat(40), Some(metadata)).await;
    let active = start_and_activate(&fixture, &"c".repeat(40), None).await;
    fixture
        .db()
        .update_git_repository_source_settings(
            fixture.group_id,
            fixture.repository_key,
            &GitRepositorySettingsUpdate {
                target_ref: None,
                pinned_commit_action: "keep",
                pinned_commit_value: None,
                index_profile: None,
                include_patterns: &[],
                exclude_patterns: &[],
                interval_seconds: None,
                webhook_enabled: false,
                enabled: false,
                retention_count: 1,
            },
        )
        .await
        .expect("disable the manual source")
        .expect("the fixture owns the source");
    let disabled = source_row(&fixture).await;
    assert!(!disabled.enabled);
    assert_eq!(disabled.interval_seconds, None);

    let failures = prune_generations(
        fixture.db(),
        fixture.group_id,
        fixture.repository_key,
        1,
        None,
    )
    .await
    .expect("the destination is unreachable without a configured Qdrant URL");
    assert_eq!(failures, 1);
    sqlx::query(
        "UPDATE context69.git_repository_generations \
         SET cleanup_next_retry_at = now() - interval '1 second' \
         WHERE repository_key = $1 AND cleanup_pending",
    )
    .bind(fixture.repository_key)
    .execute(fixture.db().pool())
    .await
    .expect("age the bounded retry so the maintenance pass is due");
    let pending = fixture
        .db()
        .list_pending_git_generation_cleanup_sources(100)
        .await
        .expect("list pending generation cleanup");
    let entry = pending
        .into_iter()
        .find(|(_, repository_key, _)| *repository_key == fixture.repository_key)
        .expect("a disabled manual source is still maintained");
    assert_eq!(entry.0, fixture.group_id);
    assert_eq!(entry.2, 1);

    run_pending_generation_cleanup(
        fixture.db(),
        vec![entry],
        |_group_id, _repository_key, _generation_key, _metadata| async move {
            Ok::<(), anyhow::Error>(())
        },
    )
    .await
    .expect("the independent pass retries without a new index");

    let pending = fixture
        .db()
        .list_pending_git_generation_cleanup_sources(100)
        .await
        .expect("list pending generation cleanup after retry");
    assert!(
        !pending
            .iter()
            .any(|(_, repository_key, _)| *repository_key == fixture.repository_key),
        "the recovered cleanup clears the pending state"
    );
    let generations = fixture
        .db()
        .list_git_repository_generations(fixture.group_id, fixture.repository_key)
        .await
        .expect("read generations");
    assert!(
        !generations
            .iter()
            .any(|generation| generation.generation_key == retired),
        "the cleaned retired generation is removed without another index"
    );
    assert!(
        generations
            .iter()
            .any(|generation| generation.generation_key == active),
        "the active generation is never removed"
    );
    assert!(
        !source_row(&fixture).await.enabled,
        "the source stays disabled"
    );

    fixture.cleanup().await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_failed_vector_cleanup_keeps_the_generation_for_retry() {
    let fixture = Fixture::setup().await;
    fixture
        .db()
        .update_git_repository_source_settings(
            fixture.group_id,
            fixture.repository_key,
            &GitRepositorySettingsUpdate {
                target_ref: None,
                pinned_commit_action: "keep",
                pinned_commit_value: None,
                index_profile: None,
                include_patterns: &[],
                exclude_patterns: &[],
                interval_seconds: None,
                webhook_enabled: false,
                enabled: true,
                retention_count: 1,
            },
        )
        .await
        .expect("set retention to one")
        .expect("the fixture owns the source");

    let metadata = || NewGitGenerationCodeVectors {
        fingerprint: "git-code-fingerprint".to_string(),
        endpoint_identity: "git-code-endpoint-identity".to_string(),
        dimensions: 8,
        collection_name: "git_code_retention_test".to_string(),
        point_count: 1,
    };
    let oldest = start_and_activate(&fixture, &"a".repeat(40), None).await;
    let vector_backed = start_and_activate(&fixture, &"b".repeat(40), Some(metadata())).await;
    let retained = start_and_activate(&fixture, &"c".repeat(40), None).await;
    let active = start_and_activate(&fixture, &"d".repeat(40), None).await;
    assert!(
        fixture
            .db()
            .get_git_generation_code_vectors(
                fixture.group_id,
                fixture.repository_key,
                vector_backed
            )
            .await
            .expect("read recorded metadata")
            .is_some(),
        "the vector-backed generation carries its recorded destination"
    );

    let failures = prune_generations(
        fixture.db(),
        fixture.group_id,
        fixture.repository_key,
        1,
        None,
    )
    .await
    .expect("prune the retired generations");
    assert_eq!(
        failures, 1,
        "the vector-backed generation cannot be cleaned without its destination"
    );

    let generations = fixture
        .db()
        .list_git_repository_generations(fixture.group_id, fixture.repository_key)
        .await
        .expect("read generations");
    assert!(
        !generations
            .iter()
            .any(|generation| generation.generation_key == oldest),
        "a generation with no remote points is removed"
    );
    let kept = generations
        .iter()
        .find(|generation| generation.generation_key == vector_backed)
        .expect("the failed cleanup keeps its metadata");
    assert_eq!(kept.status, GitGenerationStatus::Superseded);
    assert!(
        !generations
            .iter()
            .any(|generation| generation.generation_key == retained),
        "retention counts the active snapshot, so count=1 keeps only the active generation"
    );
    let (pending, error): (bool, Option<String>) = sqlx::query_as(
        "SELECT cleanup_pending, cleanup_error FROM context69.git_repository_generations \
         WHERE generation_key = $1",
    )
    .bind(vector_backed)
    .fetch_one(fixture.db().pool())
    .await
    .expect("read cleanup state");
    assert!(
        pending,
        "the row records that cleanup still owes remote work"
    );
    assert!(
        error
            .as_deref()
            .is_some_and(|error| error.contains("git_code_qdrant_url_missing")),
        "the retained row names the bounded cleanup failure"
    );
    let active_generation = fixture
        .db()
        .get_git_active_generation(fixture.group_id, fixture.repository_key)
        .await
        .expect("read active generation")
        .expect("the newest generation is active");
    assert_eq!(active_generation.generation_key, active);
    assert!(
        !fixture
            .db()
            .get_git_generation_code_vectors(
                fixture.group_id,
                fixture.repository_key,
                vector_backed
            )
            .await
            .expect("read retained metadata")
            .is_none(),
        "the retry keeps the only record of the remote points"
    );

    fixture.cleanup().await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn cleanup_fails_closed_when_the_recorded_endpoint_does_not_match() {
    let fixture = Fixture::setup_with_profile(GitIndexProfile::Hybrid).await;
    let recorded_endpoint = "http://qdrant-a.invalid:6334";
    let generation = fixture
        .db()
        .start_git_repository_generation(
            fixture.group_id,
            fixture.repository_key,
            &NewGitRepositoryGeneration {
                ref_name: "refs/heads/main".to_string(),
                commit_sha: "e".repeat(40),
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
            fixture.repository_key,
            generation,
            &NewGitGenerationCodeVectors {
                fingerprint: "git-code-endpoint-fingerprint".to_string(),
                endpoint_identity: code_endpoint_identity(recorded_endpoint),
                dimensions: 8,
                collection_name: "git_code_endpoint_identity_test".to_string(),
                point_count: 3,
            },
        )
        .await
        .expect("record the provisional destination");
    let metadata = fixture
        .db()
        .get_git_generation_code_vectors(fixture.group_id, fixture.repository_key, generation)
        .await
        .expect("read the recorded destination")
        .expect("the generation owns its destination");

    let error = remove_generation_vectors(
        fixture.group_id,
        fixture.repository_key,
        generation,
        Some(&metadata),
        Some("http://qdrant-b.invalid:6334"),
    )
    .await
    .expect_err("points on another endpoint must never be deleted from this one");
    assert!(
        error
            .to_string()
            .contains("git_code_endpoint_identity_mismatch"),
        "{error}"
    );
    assert!(
        fixture
            .db()
            .get_git_generation_code_vectors(fixture.group_id, fixture.repository_key, generation)
            .await
            .expect("read retained metadata")
            .is_some(),
        "a mismatched endpoint retains the only record of the old points"
    );

    sqlx::query(
        "UPDATE context69.git_generation_code_vectors \
         SET endpoint_identity = NULL WHERE generation_key = $1",
    )
    .bind(generation)
    .execute(fixture.db().pool())
    .await
    .expect("simulate a row recorded before the identity existed");
    let legacy = fixture
        .db()
        .get_git_generation_code_vectors(fixture.group_id, fixture.repository_key, generation)
        .await
        .expect("read the legacy destination")
        .expect("the legacy row still exists");
    let error = remove_generation_vectors(
        fixture.group_id,
        fixture.repository_key,
        generation,
        Some(&legacy),
        Some(recorded_endpoint),
    )
    .await
    .expect_err("an unknown endpoint identity fails closed");
    assert!(
        error
            .to_string()
            .contains("git_code_endpoint_identity_unknown"),
        "{error}"
    );

    // Back on the recorded endpoint the identity guard passes, so the fake
    // remote is attempted and fails instead of returning an identity conflict.
    let error = remove_generation_vectors(
        fixture.group_id,
        fixture.repository_key,
        generation,
        Some(&metadata),
        Some(recorded_endpoint),
    )
    .await
    .expect_err("the fake remote is unreachable");
    assert!(
        !error.to_string().contains("git_code_endpoint_identity"),
        "a matching endpoint proceeds to remote cleanup: {error}"
    );

    fixture.cleanup().await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn deletion_fences_new_work_and_outlives_a_concurrent_edit() {
    let fixture = Fixture::setup().await;
    let user_id = seed_user(fixture.db()).await;
    let payloads = [json!({ "repository_key": fixture.repository_key.to_string() })];
    let (task_id, _) = submit_git_index(&fixture, user_id, &payloads)
        .await
        .expect("admit the active task");

    fixture
        .db()
        .mark_git_repository_deletion_requested(fixture.group_id, fixture.repository_key)
        .await
        .expect("mark the source for deletion");
    let pending = source_row(&fixture).await;
    assert!(!pending.enabled);
    assert_eq!(pending.index_status, GitIndexStatus::Disabled);
    assert!(
        pending.deletion_requested_at.is_some(),
        "the source records that cleanup still owns its vectors"
    );
    assert!(
        !fixture
            .db()
            .request_git_refresh(fixture.group_id, fixture.repository_key)
            .await
            .expect("a disabled source records no demand"),
        "a deletion-pending source records no refresh demand"
    );

    let conflict = submit_git_index(&fixture, user_id, &payloads).await;
    assert!(
        conflict.is_err_and(|error| {
            matches!(
                error.downcast_ref::<DomainError>(),
                Some(DomainError::Conflict(code)) if code == "git_repository_not_admitting_work"
            )
        }),
        "a deletion-pending source cannot admit another task"
    );

    let edited = fixture
        .db()
        .update_git_repository_source_settings(
            fixture.group_id,
            fixture.repository_key,
            &GitRepositorySettingsUpdate {
                target_ref: None,
                pinned_commit_action: "keep",
                pinned_commit_value: None,
                index_profile: None,
                include_patterns: &[],
                exclude_patterns: &[],
                interval_seconds: None,
                webhook_enabled: false,
                enabled: true,
                retention_count: 2,
            },
        )
        .await
        .expect("a concurrent edit is applied")
        .expect("the source still exists");
    assert!(edited.enabled, "the edit re-enabled the source");
    assert!(
        edited.deletion_requested_at.is_some(),
        "the edit cannot cancel cleanup ownership"
    );
    assert!(
        !pending_deletion_contains(&fixture).await,
        "cleanup waits while the active task can still write"
    );

    sqlx::query(
        "UPDATE context69.tasks SET status = 'succeeded', finished_at = now() WHERE id = $1",
    )
    .bind(task_id)
    .execute(fixture.db().pool())
    .await
    .expect("finish the active task");
    assert!(
        pending_deletion_contains(&fixture).await,
        "cleanup resumes once no writer is active"
    );

    assert!(
        fixture
            .db()
            .delete_git_repository_source(fixture.group_id, fixture.repository_key)
            .await
            .expect("delete the source after cleanup"),
        "the delete owns the row even after a concurrent edit"
    );
    assert!(
        fixture
            .db()
            .get_git_repository_source(fixture.group_id, fixture.repository_key)
            .await
            .expect("read the deleted source")
            .is_none(),
        "the source and its cascade are gone"
    );

    let db = fixture.db().clone();
    fixture.cleanup().await;
    sqlx::query("DELETE FROM context69.users WHERE id = $1")
        .bind(user_id)
        .execute(db.pool())
        .await
        .expect("clean up the task user");
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn an_old_building_generation_is_deferred_while_the_lifetime_guard_is_held() {
    let fixture = Fixture::setup().await;
    let building = fixture
        .db()
        .start_git_repository_generation(
            fixture.group_id,
            fixture.repository_key,
            &NewGitRepositoryGeneration {
                ref_name: "refs/heads/main".to_string(),
                commit_sha: "9".repeat(40),
                index_profile: GitIndexProfile::Hybrid,
            },
        )
        .await
        .expect("start a building generation")
        .generation_key;
    sqlx::query(
        "UPDATE context69.git_repository_generations \
         SET started_at = now() - interval '2 hours' \
         WHERE generation_key = $1",
    )
    .bind(building)
    .execute(fixture.db().pool())
    .await
    .expect("age the building generation past the orphan threshold");

    let guard = GitIndexLifetimeGuard::acquire(fixture.db().pool(), fixture.repository_key)
        .await
        .expect("hold the index lifetime lock");
    let failures = prune_generations(
        fixture.db(),
        fixture.group_id,
        fixture.repository_key,
        2,
        None,
    )
    .await
    .expect("cleanup defers instead of racing the guard");
    assert_eq!(failures, 0);
    let generations = fixture
        .db()
        .list_git_repository_generations(fixture.group_id, fixture.repository_key)
        .await
        .expect("read generations while the guard is held");
    assert!(
        generations
            .iter()
            .any(|generation| generation.generation_key == building),
        "age alone never removes a generation that a real index guard still owns"
    );

    guard.release().await.expect("release the lifetime lock");
    let failures = prune_generations(
        fixture.db(),
        fixture.group_id,
        fixture.repository_key,
        2,
        None,
    )
    .await
    .expect("cleanup runs after the guard is released");
    assert_eq!(failures, 0);
    let generations = fixture
        .db()
        .list_git_repository_generations(fixture.group_id, fixture.repository_key)
        .await
        .expect("read generations after release");
    assert!(
        !generations
            .iter()
            .any(|generation| generation.generation_key == building),
        "a released guard leaves the building generation as an orphan to clean"
    );

    fixture.cleanup().await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn an_abandoned_building_generation_is_scheduled_and_defers_while_guarded() {
    let fixture = Fixture::setup_with_profile(GitIndexProfile::Hybrid).await;
    let building = fixture
        .db()
        .start_git_repository_generation(
            fixture.group_id,
            fixture.repository_key,
            &NewGitRepositoryGeneration {
                ref_name: "refs/heads/main".to_string(),
                commit_sha: "d".repeat(40),
                index_profile: GitIndexProfile::Hybrid,
            },
        )
        .await
        .expect("start a generation that will be abandoned")
        .generation_key;
    fixture
        .db()
        .record_git_generation_code_vectors(
            fixture.group_id,
            fixture.repository_key,
            building,
            &NewGitGenerationCodeVectors {
                fingerprint: "git-code-abandoned".to_string(),
                endpoint_identity: code_endpoint_identity("http://qdrant-abandoned.invalid"),
                dimensions: 8,
                collection_name: "git_code_abandoned_test".to_string(),
                point_count: 1,
            },
        )
        .await
        .expect("record the abandoned destination");
    sqlx::query(
        "UPDATE context69.git_repository_generations \
         SET started_at = now() - interval '2 hours' \
         WHERE generation_key = $1",
    )
    .bind(building)
    .execute(fixture.db().pool())
    .await
    .expect("age the abandoned building generation");
    let deferred_until: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
        "SELECT cleanup_deferred_until FROM context69.git_repository_sources \
         WHERE repository_key = $1",
    )
    .bind(fixture.repository_key)
    .fetch_one(fixture.db().pool())
    .await
    .expect("read the source deferral");
    assert!(
        deferred_until.is_none(),
        "the source starts without a scheduler deferral"
    );

    let due = fixture
        .db()
        .list_pending_git_generation_cleanup_sources(100)
        .await
        .expect("list due sources")
        .into_iter()
        .find(|(_, repository_key, _)| *repository_key == fixture.repository_key)
        .expect("an abandoned building generation without a pending flag is due on age");

    let guard = GitIndexLifetimeGuard::acquire(fixture.db().pool(), fixture.repository_key)
        .await
        .expect("hold the index lifetime lock");
    run_pending_generation_cleanup(
        fixture.db(),
        vec![due],
        |_group_id, _repository_key, _generation_key, _metadata| async move {
            Ok::<(), anyhow::Error>(())
        },
    )
    .await
    .expect("a guard-held source defers without failing");
    let deferred = fixture
        .db()
        .list_pending_git_generation_cleanup_sources(100)
        .await
        .expect("list due sources while guarded");
    assert!(
        !deferred
            .iter()
            .any(|(_, repository_key, _)| *repository_key == fixture.repository_key),
        "a guard-held source does not consume the next bounded batch"
    );
    let deferred_until: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
        "SELECT cleanup_deferred_until FROM context69.git_repository_sources \
         WHERE repository_key = $1",
    )
    .bind(fixture.repository_key)
    .fetch_one(fixture.db().pool())
    .await
    .expect("read the scheduler deferral");
    assert!(
        deferred_until.is_some(),
        "the scheduler-only deferral is recorded"
    );
    assert!(
        fixture
            .db()
            .list_git_repository_generations(fixture.group_id, fixture.repository_key)
            .await
            .expect("read generations while guarded")
            .iter()
            .any(|generation| generation.generation_key == building),
        "the abandoned generation is untouched while its guard is held"
    );

    guard.release().await.expect("release the lifetime lock");
    sqlx::query(
        "UPDATE context69.git_repository_sources \
         SET cleanup_deferred_until = now() - interval '1 second' \
         WHERE repository_key = $1",
    )
    .bind(fixture.repository_key)
    .execute(fixture.db().pool())
    .await
    .expect("age the scheduler deferral");
    let due = fixture
        .db()
        .list_pending_git_generation_cleanup_sources(100)
        .await
        .expect("list due sources after release")
        .into_iter()
        .find(|(_, repository_key, _)| *repository_key == fixture.repository_key)
        .expect("the abandoned generation is due again");
    run_pending_generation_cleanup(
        fixture.db(),
        vec![due],
        |_group_id, _repository_key, _generation_key, _metadata| async move {
            Ok::<(), anyhow::Error>(())
        },
    )
    .await
    .expect("the abandoned generation is cleaned after release");
    assert!(
        !fixture
            .db()
            .list_git_repository_generations(fixture.group_id, fixture.repository_key)
            .await
            .expect("read generations after release")
            .iter()
            .any(|generation| generation.generation_key == building),
        "a released guard leaves the abandoned building generation as an orphan to clean"
    );

    fixture.cleanup().await;
}

async fn seed_pending_cleanup_source(fixture: &Fixture, label: &str) -> Uuid {
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

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn more_than_one_batch_of_failing_sources_does_not_starve_a_recoverable_one() {
    let fixture = Fixture::setup().await;
    let mut seeded = Vec::new();
    for index in 0..17 {
        seeded.push(seed_pending_cleanup_source(&fixture, &format!("fair-{index}")).await);
    }

    let first = fixture
        .db()
        .list_pending_git_generation_cleanup_sources(16)
        .await
        .expect("list the first bounded batch");
    assert_eq!(first.len(), 16, "one bounded batch selects sixteen sources");
    let first_keys: Vec<Uuid> = first
        .iter()
        .map(|(_, repository_key, _)| *repository_key)
        .collect();
    let recoverable = *seeded
        .iter()
        .find(|key| !first_keys.contains(key))
        .expect("one source is outside the first batch");

    run_pending_generation_cleanup(
        fixture.db(),
        first,
        |_group_id, _repository_key, _generation_key, _metadata| async move {
            Err::<(), anyhow::Error>(anyhow::anyhow!("git_code_sink_failed"))
        },
    )
    .await
    .expect("the first pass records a bounded retry for every failure");

    let second = fixture
        .db()
        .list_pending_git_generation_cleanup_sources(16)
        .await
        .expect("list the next bounded batch");
    assert!(
        second
            .iter()
            .any(|(_, repository_key, _)| *repository_key == recoverable),
        "backed-off permanent failures cannot starve a due recoverable source"
    );

    run_pending_generation_cleanup(
        fixture.db(),
        second,
        |_group_id, _repository_key, _generation_key, _metadata| async move {
            Ok::<(), anyhow::Error>(())
        },
    )
    .await
    .expect("the recoverable source is cleaned");

    let pending = fixture
        .db()
        .list_pending_git_generation_cleanup_sources(100)
        .await
        .expect("list pending cleanup after the recovery pass");
    assert!(
        !pending
            .iter()
            .any(|(_, repository_key, _)| *repository_key == recoverable),
        "the recovered source is no longer pending"
    );

    fixture.cleanup().await;
}

async fn seed_pending_deletion_source(fixture: &Fixture, label: &str) -> Uuid {
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

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn more_than_one_batch_of_failing_deletions_does_not_starve_a_recoverable_one() {
    let fixture = Fixture::setup().await;
    let mut seeded = Vec::new();
    for index in 0..17 {
        seeded.push(seed_pending_deletion_source(&fixture, &format!("del-fair-{index}")).await);
    }

    let first = fixture
        .db()
        .list_pending_git_deletions(16)
        .await
        .expect("list the first bounded deletion batch");
    assert_eq!(first.len(), 16, "one bounded batch selects sixteen sources");
    let first_keys: Vec<Uuid> = first
        .iter()
        .map(|(_, repository_key)| *repository_key)
        .collect();
    let recoverable = *seeded
        .iter()
        .find(|key| !first_keys.contains(key))
        .expect("one source is outside the first batch");

    for (group_id, repository_key) in &first {
        let error = cleanup_deleted_source(
            fixture.db(),
            *group_id,
            *repository_key,
            |_generation_key, _metadata| async move {
                Err::<(), anyhow::Error>(anyhow::anyhow!("git_code_sink_failed"))
            },
        )
        .await
        .expect_err("every first-batch cleanup fails");
        assert!(error.to_string().contains("git_code_sink_failed"));
    }
    assert!(
        fixture
            .db()
            .get_git_repository_source(fixture.group_id, recoverable)
            .await
            .expect("read the deferred source")
            .is_some(),
        "a failed deletion keeps its source row"
    );

    let second = fixture
        .db()
        .list_pending_git_deletions(16)
        .await
        .expect("list the next bounded deletion batch");
    assert!(
        second
            .iter()
            .any(|(_, repository_key)| *repository_key == recoverable),
        "backed-off permanent failures cannot starve a due recoverable deletion"
    );

    for (group_id, repository_key) in &second {
        let deleted = cleanup_deleted_source(
            fixture.db(),
            *group_id,
            *repository_key,
            |_generation_key, _metadata| async move { Ok::<(), anyhow::Error>(()) },
        )
        .await
        .expect("the recoverable deletion is cleaned")
        .expect("the lifetime guard is free");
        assert!(deleted, "the recoverable source is removed");
    }
    assert!(
        fixture
            .db()
            .get_git_repository_source(fixture.group_id, recoverable)
            .await
            .expect("read the deleted source")
            .is_none(),
        "the recoverable deletion completed"
    );
    assert!(
        !fixture
            .db()
            .list_pending_git_deletions(100)
            .await
            .expect("list pending deletions after recovery")
            .iter()
            .any(|(_, repository_key)| *repository_key == recoverable),
        "the recovered source is no longer pending"
    );

    fixture.cleanup().await;
}

#[test]
fn a_repository_key_is_declared_only_by_a_single_well_formed_payload() {
    use crate::db::task_git_dedup::declared_git_repository_key;

    let key = Uuid::new_v4();
    assert_eq!(
        declared_git_repository_key(&[json!({ "repository_key": key.to_string() })]),
        Some(key)
    );
    assert!(declared_git_repository_key(&[]).is_none());
    assert!(
        declared_git_repository_key(&[json!({ "repository_key": key.to_string() }), json!({})])
            .is_none(),
        "a task names one repository or none"
    );
    assert!(declared_git_repository_key(&[json!({ "repository_key": "not-a-uuid" })]).is_none());
}
