//! Deletion fencing, fair deletion retries, and payload declaration.

use serde_json::json;
use uuid::Uuid;

use crate::contracts::sources::GitIndexStatus;
use crate::db::GitRepositorySettingsUpdate;
use crate::domain_errors::DomainError;

use super::super::super::support::Fixture;
use super::super::cleanup_deleted_source;
use super::support::{
    pending_deletion_contains, seed_pending_deletion_source, source_row, submit_git_index,
};

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn deletion_fences_new_work_and_outlives_a_concurrent_edit() {
    let fixture = Fixture::setup().await;
    let user_id = fixture.seed_user().await;
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
