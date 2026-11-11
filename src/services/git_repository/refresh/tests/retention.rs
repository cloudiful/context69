//! Retention, pruning, abandoned-building cleanup, and cleanup fairness.

use uuid::Uuid;

use crate::contracts::sources::GitIndexProfile;
use crate::db::{
    GitIndexLifetimeGuard, GitRepositorySettingsUpdate, NewGitGenerationCodeVectors,
    NewGitRepositoryGeneration,
};
use crate::services::git_repository::code_vectors::code_endpoint_identity;

use super::super::super::support::Fixture;
use super::super::{prune_generations, run_pending_generation_cleanup};
use super::support::{seed_pending_cleanup_source, source_row, start_and_activate};

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
