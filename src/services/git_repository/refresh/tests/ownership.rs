//! Remote vector cleanup failures and endpoint identity ownership.

use crate::contracts::sources::{GitGenerationStatus, GitIndexProfile};
use crate::db::{
    GitRepositorySettingsUpdate, NewGitGenerationCodeVectors, NewGitRepositoryGeneration,
};
use crate::services::git_repository::code_vectors::code_endpoint_identity;

use super::super::super::support::Fixture;
use super::super::{prune_generations, remove_generation_vectors};
use super::support::start_and_activate;

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
