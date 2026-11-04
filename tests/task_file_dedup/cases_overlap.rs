//! Issue #400 overlap regressions: atomic partial-overlap rejection,
//! idempotent reuse, and reprocessing of failed files.
//!
//! Runs only when CONTEXT69_TEST_DATABASE_URL points to a scratch database.

use context69::db::{CreateTaskSubmissionRequest, Database};
use serde_json::json;
use uuid::Uuid;

use crate::dedup_support::count_active_file_items;
use crate::support::{
    add_group_maintainer, cleanup_file, cleanup_group, cleanup_task, cleanup_user, connect_scratch,
    create_file_task, insert_file, insert_file_in_group, seed_group, seed_test_user,
};

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn partially_overlapping_batch_is_rejected_atomically() {
    let db = connect_scratch().await;
    let user_id = seed_test_user(&db).await;
    let group_id = seed_group(&db).await;
    let active_file = insert_file_in_group(&db, group_id, "failed").await;
    let free_file = insert_file_in_group(&db, group_id, "failed").await;
    let (active_task, _) =
        create_file_task(&db, user_id, group_id, active_file, "partial-active").await;

    let payloads = [
        json!({ "file_id": active_file }),
        json!({ "file_id": free_file }),
    ];
    let error = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id: Uuid::new_v4(),
            user_id,
            group_id: Some(group_id),
            kind: "file_batch",
            group_path: Some("test/file-status"),
            source_key: None,
            payloads: &payloads,
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: "partial-overlap",
        })
        .await
        .expect_err("partially overlapping batch must be rejected");
    assert!(
        error.to_string().contains("conflict"),
        "rejection must map to the existing conflict response: {error}"
    );
    assert_eq!(
        count_active_file_items(&db, active_file).await,
        1,
        "the active file must keep exactly one active item"
    );
    assert_eq!(
        count_active_file_items(&db, free_file).await,
        0,
        "the free file must not be scheduled by a rejected batch"
    );

    cleanup_task(&db, active_task).await;
    cleanup_file(&db, active_file).await;
    cleanup_file(&db, free_file).await;
    cleanup_group(&db, group_id).await;
    cleanup_user(&db, user_id).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn resubmitting_active_file_reuses_existing_task() {
    let db = connect_scratch().await;
    let user_id = seed_test_user(&db).await;
    let (file_id, group_id) = insert_file(&db, "failed").await;
    let (task_a, _) = create_file_task(&db, user_id, group_id, file_id, "dedup-first").await;

    let (task_id, reused, item_ids) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id: Uuid::new_v4(),
            user_id,
            group_id: Some(group_id),
            kind: "file_batch",
            group_path: Some("test/file-status"),
            source_key: None,
            payloads: &[json!({ "file_id": file_id })],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: "dedup-second",
        })
        .await
        .expect("resubmit active file");
    assert!(reused, "resubmitting an active file must reuse its task");
    assert_eq!(task_id, task_a, "reused task must be the active one");
    assert_eq!(item_ids.len(), 1, "reuse must not create duplicate items");
    assert_eq!(
        count_active_file_items(&db, file_id).await,
        1,
        "a file may have at most one active processing item"
    );

    cleanup_task(&db, task_a).await;
    cleanup_file(&db, file_id).await;
    cleanup_group(&db, group_id).await;
    cleanup_user(&db, user_id).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn reprocess_creates_new_task_for_failed_files() {
    let db = connect_scratch().await;
    let user_id = seed_test_user(&db).await;
    for status in ["failed"] {
        let (file_id, group_id) = insert_file(&db, status).await;
        let (task_id, reused, item_ids) = db
            .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
                task_id: Uuid::new_v4(),
                user_id,
                group_id: Some(group_id),
                kind: "file_batch",
                group_path: Some("test/file-status"),
                source_key: None,
                payloads: &[json!({ "file_id": file_id })],
                input_storage_object_ids: None,
                idempotency_key: None,
                request_hash: &format!("reprocess-{status}"),
            })
            .await
            .expect("create reprocess task");
        assert!(
            !reused,
            "{status} file with no active item must create a new task"
        );
        assert_eq!(item_ids.len(), 1);
        let item_file: Option<Uuid> =
            sqlx::query_scalar("SELECT file_id FROM context69.task_items WHERE id = $1")
                .bind(item_ids[0])
                .fetch_one(db.pool())
                .await
                .expect("load item file id");
        assert_eq!(
            item_file,
            Some(file_id),
            "{status} file item must carry the file_id"
        );

        cleanup_task(&db, task_id).await;
        cleanup_file(&db, file_id).await;
        cleanup_group(&db, group_id).await;
    }
    cleanup_user(&db, user_id).await;
}

async fn submit(
    db: &Database,
    user_id: i64,
    group_id: Option<i64>,
    kind: &str,
    payloads: &[serde_json::Value],
    task_id: Uuid,
    idempotency_key: Option<&str>,
    request_hash: &str,
) -> anyhow::Result<(Uuid, bool, Vec<Uuid>)> {
    db.create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
        task_id,
        user_id,
        group_id,
        kind,
        group_path: Some("test/file-status"),
        source_key: None,
        payloads,
        input_storage_object_ids: None,
        idempotency_key,
        request_hash,
    })
    .await
}

async fn bound_idempotency(db: &Database, user_id: i64, key: &str) -> (Uuid, String) {
    let row = sqlx::query_file!("src/sql/db/tasks/idempotency_get.sql", user_id, key)
        .fetch_one(db.pool())
        .await
        .expect("read the idempotency binding");
    (row.task_id, row.request_hash)
}

async fn remove_idempotency_keys(db: &Database, user_id: i64) {
    sqlx::query_file!(
        "tests/sql/task_execution/delete_user_idempotency_keys.sql",
        user_id
    )
    .execute(db.pool())
    .await
    .expect("delete idempotency keys");
}

fn is_conflict(error: &anyhow::Error) -> bool {
    matches!(
        context69::domain_errors::find_domain_error(error),
        Some(context69::domain_errors::DomainError::Conflict(_))
    )
}

/// A fresh idempotency key on an active-file dedup reuse must bind the key,
/// hash and accepted task inside the reuse transaction.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn reusing_an_active_file_binds_a_fresh_idempotency_key() {
    let db = connect_scratch().await;
    let user_id = seed_test_user(&db).await;
    let group_id = seed_group(&db).await;
    let file_id = insert_file_in_group(&db, group_id, "failed").await;
    let (active_task, active_items) =
        create_file_task(&db, user_id, group_id, file_id, "bind-active").await;
    let key = format!("overlap-bind-{}", Uuid::new_v4());
    let candidate = Uuid::new_v4();
    let payloads = [json!({ "file_id": file_id })];

    let (task_id, reused, item_ids) = submit(
        &db,
        user_id,
        Some(group_id),
        "file_batch",
        &payloads,
        candidate,
        Some(&key),
        "bind-hash",
    )
    .await
    .expect("dedup reuse accepts a fresh idempotency key");

    assert!(reused, "the active file item is reused");
    assert_eq!(task_id, active_task);
    assert_eq!(item_ids, active_items);
    assert_eq!(
        bound_idempotency(&db, user_id, &key).await,
        (active_task, "bind-hash".to_string())
    );
    assert!(
        db.get_task_internal(candidate).await.unwrap().is_none(),
        "binding a reuse key never leaves the candidate task behind"
    );

    remove_idempotency_keys(&db, user_id).await;
    cleanup_task(&db, active_task).await;
    cleanup_file(&db, file_id).await;
    cleanup_group(&db, group_id).await;
    cleanup_user(&db, user_id).await;
}

/// A replay returns the bound task after it is terminal, while a changed body
/// under the same key conflicts even when the dedup candidate is the same task.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn replay_after_terminal_returns_the_bound_task_and_changed_hashes_conflict() {
    let db = connect_scratch().await;
    let user_id = seed_test_user(&db).await;
    let group_id = seed_group(&db).await;
    add_group_maintainer(&db, group_id, user_id).await;
    let file_id = insert_file_in_group(&db, group_id, "failed").await;
    let (active_task, active_items) =
        create_file_task(&db, user_id, group_id, file_id, "replay-active").await;
    let key = format!("overlap-replay-{}", Uuid::new_v4());
    let payloads = [json!({ "file_id": file_id })];
    let (task_id, reused, item_ids) = submit(
        &db,
        user_id,
        Some(group_id),
        "file_batch",
        &payloads,
        Uuid::new_v4(),
        Some(&key),
        "replay-hash",
    )
    .await
    .expect("the first reuse binds the key");
    assert!(reused);
    assert_eq!(task_id, active_task);
    assert_eq!(item_ids, active_items);

    assert!(
        db.cancel_task(active_task, user_id).await.unwrap(),
        "the fixture task reaches a terminal status"
    );

    let (replayed_task, reused, replayed_items) = submit(
        &db,
        user_id,
        Some(group_id),
        "file_batch",
        &payloads,
        Uuid::new_v4(),
        Some(&key),
        "replay-hash",
    )
    .await
    .expect("a same-body replay returns the original task after it is terminal");
    assert!(reused);
    assert_eq!(replayed_task, active_task);
    assert_eq!(replayed_items, active_items);

    let same_file_changed_body = [json!({ "file_id": file_id, "marker": "changed" })];
    let error = submit(
        &db,
        user_id,
        Some(group_id),
        "file_batch",
        &same_file_changed_body,
        Uuid::new_v4(),
        Some(&key),
        "replay-other-hash",
    )
    .await
    .expect_err("a changed body conflicts before the task id is compared");
    assert!(
        is_conflict(&error),
        "a changed body is a typed conflict: {error}"
    );

    let other_file = insert_file_in_group(&db, group_id, "failed").await;
    let other_payloads = [json!({ "file_id": other_file })];
    let error = submit(
        &db,
        user_id,
        Some(group_id),
        "file_batch",
        &other_payloads,
        Uuid::new_v4(),
        Some(&key),
        "replay-other-file",
    )
    .await
    .expect_err("a different file under the key conflicts too");
    assert!(
        is_conflict(&error),
        "a different body is a typed conflict: {error}"
    );

    remove_idempotency_keys(&db, user_id).await;
    cleanup_task(&db, active_task).await;
    cleanup_file(&db, file_id).await;
    cleanup_file(&db, other_file).await;
    cleanup_group(&db, group_id).await;
    cleanup_user(&db, user_id).await;
}

/// Concurrent reuse submissions with one key agree on one accepted task and one
/// binding, and never leave a candidate task.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn concurrent_reuse_bindings_agree_on_one_task() {
    let db = connect_scratch().await;
    let user_id = seed_test_user(&db).await;
    let group_id = seed_group(&db).await;
    let file_id = insert_file_in_group(&db, group_id, "failed").await;
    let (active_task, active_items) =
        create_file_task(&db, user_id, group_id, file_id, "concurrent-active").await;
    let key = format!("overlap-concurrent-{}", Uuid::new_v4());
    let payloads = [json!({ "file_id": file_id })];
    let first_candidate = Uuid::new_v4();
    let second_candidate = Uuid::new_v4();

    let (first, second) = tokio::join!(
        submit(
            &db,
            user_id,
            Some(group_id),
            "file_batch",
            &payloads,
            first_candidate,
            Some(&key),
            "concurrent-hash",
        ),
        submit(
            &db,
            user_id,
            Some(group_id),
            "file_batch",
            &payloads,
            second_candidate,
            Some(&key),
            "concurrent-hash",
        ),
    );
    for result in [first, second] {
        let (task_id, reused, item_ids) = result.expect("both reuse submissions succeed");
        assert!(reused);
        assert_eq!(task_id, active_task);
        assert_eq!(item_ids, active_items);
    }
    assert_eq!(
        bound_idempotency(&db, user_id, &key).await,
        (active_task, "concurrent-hash".to_string())
    );
    assert!(
        db.get_task_internal(first_candidate)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        db.get_task_internal(second_candidate)
            .await
            .unwrap()
            .is_none()
    );

    remove_idempotency_keys(&db, user_id).await;
    cleanup_task(&db, active_task).await;
    cleanup_file(&db, file_id).await;
    cleanup_group(&db, group_id).await;
    cleanup_user(&db, user_id).await;
}

/// Concurrent submissions with one key but different hashes accept exactly one
/// and never overwrite the accepted hash.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn concurrent_different_hashes_never_override_the_winner() {
    let db = connect_scratch().await;
    let user_id = seed_test_user(&db).await;
    let group_id = seed_group(&db).await;
    let file_id = insert_file_in_group(&db, group_id, "failed").await;
    let (active_task, _) =
        create_file_task(&db, user_id, group_id, file_id, "hash-race-active").await;
    let key = format!("overlap-hash-race-{}", Uuid::new_v4());
    let payloads = [json!({ "file_id": file_id })];
    let first_candidate = Uuid::new_v4();
    let second_candidate = Uuid::new_v4();

    let (first, second) = tokio::join!(
        submit(
            &db,
            user_id,
            Some(group_id),
            "file_batch",
            &payloads,
            first_candidate,
            Some(&key),
            "hash-race-one",
        ),
        submit(
            &db,
            user_id,
            Some(group_id),
            "file_batch",
            &payloads,
            second_candidate,
            Some(&key),
            "hash-race-two",
        ),
    );
    let outcomes = [first, second];
    let winners = outcomes.iter().filter(|result| result.is_ok()).count();
    let losers = outcomes.iter().filter(|result| result.is_err()).count();
    assert_eq!(winners, 1, "exactly one request accepts the key");
    assert_eq!(losers, 1, "the other request conflicts");

    let (bound_task, bound_hash) = bound_idempotency(&db, user_id, &key).await;
    assert_eq!(bound_task, active_task);
    let losing_hash = if bound_hash == "hash-race-one" {
        "hash-race-two"
    } else {
        assert_eq!(bound_hash, "hash-race-two");
        "hash-race-one"
    };

    let replay = submit(
        &db,
        user_id,
        Some(group_id),
        "file_batch",
        &payloads,
        Uuid::new_v4(),
        Some(&key),
        &bound_hash,
    )
    .await
    .expect("the accepted hash replays the bound task");
    assert!(replay.1);
    assert_eq!(replay.0, active_task);
    let error = submit(
        &db,
        user_id,
        Some(group_id),
        "file_batch",
        &payloads,
        Uuid::new_v4(),
        Some(&key),
        losing_hash,
    )
    .await
    .expect_err("the losing hash never overrides the accepted one");
    assert!(is_conflict(&error));

    remove_idempotency_keys(&db, user_id).await;
    cleanup_task(&db, active_task).await;
    cleanup_file(&db, file_id).await;
    cleanup_group(&db, group_id).await;
    cleanup_user(&db, user_id).await;
}

/// A concurrent new-submission loser rolls back its candidate task and returns
/// the winner's task and item ids.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_losing_new_submission_rolls_back_and_returns_the_winner() {
    let db = connect_scratch().await;
    let user_id = seed_test_user(&db).await;
    let key = format!("overlap-new-race-{}", Uuid::new_v4());
    let payloads = [json!({ "external_id": "same-body" })];
    let first_candidate = Uuid::new_v4();
    let second_candidate = Uuid::new_v4();

    let (first, second) = tokio::join!(
        submit(
            &db,
            user_id,
            None,
            "text_batch",
            &payloads,
            first_candidate,
            Some(&key),
            "new-race-hash",
        ),
        submit(
            &db,
            user_id,
            None,
            "text_batch",
            &payloads,
            second_candidate,
            Some(&key),
            "new-race-hash",
        ),
    );
    let first = first.expect("the first submission returns the winner");
    let second = second.expect("the second submission returns the winner");
    assert_eq!(
        first.0, second.0,
        "both submissions report the one bound task"
    );
    assert_eq!(first.2, second.2, "both report the winner's item ids");

    let winner_exists = db
        .get_task_internal(first_candidate)
        .await
        .unwrap()
        .is_some();
    let second_exists = db
        .get_task_internal(second_candidate)
        .await
        .unwrap()
        .is_some();
    assert_ne!(
        winner_exists, second_exists,
        "exactly one candidate task survives the race"
    );
    let winner = if winner_exists {
        first_candidate
    } else {
        second_candidate
    };
    assert_eq!(first.0, winner);
    assert_eq!(
        bound_idempotency(&db, user_id, &key).await,
        (winner, "new-race-hash".to_string())
    );
    assert_eq!(
        db.get_task_internal(winner).await.unwrap().unwrap().id,
        winner
    );

    remove_idempotency_keys(&db, user_id).await;
    cleanup_task(&db, winner).await;
    cleanup_user(&db, user_id).await;
}
