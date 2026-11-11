//! Demand, interval, backoff, and settings-driven scheduling.

use serde_json::json;

use crate::contracts::sources::{GitIndexProfile, GitIndexStatus};
use crate::db::GitRepositorySettingsUpdate;

use super::super::super::support::Fixture;
use super::support::{due_contains, source_row, submit_git_index};

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_demand_recorded_during_a_run_survives_its_completion() {
    let fixture = Fixture::setup().await;
    let user_id = fixture.seed_user().await;
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
