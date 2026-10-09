//! Issue 734 terminal projection regressions for the optimized
//! `project_file_status.sql`.
//!
//! The optimized statement resolves its target by item id through an indexed
//! file-identity join and evaluates the active-sibling guard once. These tests
//! pin the preserved behavior: success/failure projection, succeeded-file
//! non-regression, active-sibling protection, foreign-group fence, and
//! missing-file/stale-status no-ops.
//!
//! Runs only when CONTEXT69_TEST_DATABASE_URL points to a scratch database
//! (migrations are applied automatically). They are skipped otherwise.

#[path = "task_file_status/support.rs"]
mod support;

use uuid::Uuid;

use support::{
    add_group_maintainer, cleanup_file, cleanup_group, cleanup_task, cleanup_user, connect_scratch,
    create_file_task, create_raw_active_file_task, file_status, insert_file, insert_file_in_group,
    seed_group, seed_test_user,
};

static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const PROJECTION_SQL: &str = include_str!("../src/sql/db/tasks/project_file_status.sql");

#[test]
fn optimized_projection_keeps_indexed_join_and_single_sibling_check() {
    assert!(
        PROJECTION_SQL.contains("FROM target"),
        "projection must join the file through the indexed target"
    );
    assert!(
        PROJECTION_SQL.contains("JOIN target"),
        "sibling guard must reuse the same target"
    );
    assert!(
        PROJECTION_SQL.contains("has_sibling"),
        "sibling guard must be evaluated once"
    );
    assert_eq!(
        PROJECTION_SQL.matches("sibling.status IN").count(),
        1,
        "the three repeated sibling checks must be combined into one"
    );
    assert!(
        PROJECTION_SQL.contains("file.group_id = target.group_id"),
        "group ownership fence must be preserved"
    );
    assert!(
        !PROJECTION_SQL.contains("WHERE file.id = ("),
        "outer-file correlated scalar subquery must be gone"
    );
}

async fn lease_running(db: &context69::db::Database, task_id: Uuid, item_id: Uuid) -> (Uuid, i64) {
    let lease_token = Uuid::new_v4();
    sqlx::query(
        "UPDATE context69.task_items SET status = 'running', lease_token = $2, \
         attempt_count = 1 WHERE id = $1",
    )
    .bind(item_id)
    .bind(lease_token)
    .execute(db.pool())
    .await
    .expect("lease item");
    let attempt_id: i64 = sqlx::query_scalar(
        "INSERT INTO context69.task_attempts (task_id, item_id, attempt, status) \
         VALUES ($1, $2, 1, 'running') RETURNING id",
    )
    .bind(task_id)
    .bind(item_id)
    .fetch_one(db.pool())
    .await
    .expect("insert attempt");
    (lease_token, attempt_id)
}

#[tokio::test]
async fn terminal_status_projects_success_and_failure() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    let _guard = TEST_LOCK.lock().await;
    let user_id = seed_test_user(&db).await;
    for (finish_status, expected) in [("succeeded", "succeeded"), ("failed", "failed")] {
        let (file_id, group_id) = insert_file(&db, "failed").await;
        add_group_maintainer(&db, group_id, user_id).await;
        let (task_id, item_ids) = create_file_task(
            &db,
            user_id,
            group_id,
            file_id,
            &format!("sql-opt-{expected}"),
        )
        .await;
        let (lease_token, attempt_id) = lease_running(&db, task_id, item_ids[0]).await;
        let updated = db
            .finish_task_item(context69::db::FinishTaskItemRequest {
                task_id,
                item_id: item_ids[0],
                status: finish_status,
                resource_id: None,
                failure_stage: (finish_status == "failed").then_some("storage"),
                error_message: (finish_status == "failed").then_some("boom"),
                retryable: false,
                lease_token,
                attempt_id,
            })
            .await
            .expect("finish item");
        assert!(updated);
        let (status, error, ingested_at) = file_status(&db, file_id).await;
        assert_eq!(status, expected);
        if finish_status == "failed" {
            assert_eq!(error.as_deref(), Some("boom"));
            assert!(ingested_at.is_none());
        } else {
            assert_eq!(error, None);
            assert!(ingested_at.is_some());
        }
        cleanup_task(&db, task_id).await;
        cleanup_file(&db, file_id).await;
        cleanup_group(&db, group_id).await;
    }
    cleanup_user(&db, user_id).await;
}

#[tokio::test]
async fn succeeded_file_is_never_regressed() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    let _guard = TEST_LOCK.lock().await;
    let user_id = seed_test_user(&db).await;
    let (file_id, group_id) = insert_file(&db, "succeeded").await;
    add_group_maintainer(&db, group_id, user_id).await;
    let (task_id, item_ids) =
        create_file_task(&db, user_id, group_id, file_id, "sql-opt-succeeded-guard").await;
    let (lease_token, attempt_id) = lease_running(&db, task_id, item_ids[0]).await;
    let updated = db
        .finish_task_item(context69::db::FinishTaskItemRequest {
            task_id,
            item_id: item_ids[0],
            status: "failed",
            resource_id: None,
            failure_stage: Some("storage"),
            error_message: Some("boom"),
            retryable: false,
            lease_token,
            attempt_id,
        })
        .await
        .expect("finish item");
    assert!(updated);
    let (status, _, _) = file_status(&db, file_id).await;
    assert_eq!(status, "succeeded", "a succeeded file must never regress");
    cleanup_task(&db, task_id).await;
    cleanup_file(&db, file_id).await;
    cleanup_group(&db, group_id).await;
    cleanup_user(&db, user_id).await;
}

#[tokio::test]
async fn active_sibling_blocks_projection() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    let _guard = TEST_LOCK.lock().await;
    let user_id = seed_test_user(&db).await;
    let (file_id, group_id) = insert_file(&db, "failed").await;
    add_group_maintainer(&db, group_id, user_id).await;
    let (task_a, items_a) =
        create_file_task(&db, user_id, group_id, file_id, "sql-opt-sibling-a").await;
    let (task_b, _) = create_raw_active_file_task(&db, user_id, group_id, file_id, "queued").await;
    let (lease_token, attempt_id) = lease_running(&db, task_a, items_a[0]).await;
    let updated = db
        .finish_task_item(context69::db::FinishTaskItemRequest {
            task_id: task_a,
            item_id: items_a[0],
            status: "succeeded",
            resource_id: None,
            failure_stage: None,
            error_message: None,
            retryable: false,
            lease_token,
            attempt_id,
        })
        .await
        .expect("finish item");
    assert!(updated);
    let (status, _, _) = file_status(&db, file_id).await;
    assert_eq!(status, "failed", "an active sibling must block projection");
    cleanup_task(&db, task_a).await;
    cleanup_task(&db, task_b).await;
    cleanup_file(&db, file_id).await;
    cleanup_group(&db, group_id).await;
    cleanup_user(&db, user_id).await;
}

#[tokio::test]
async fn foreign_group_file_is_never_projected() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    let _guard = TEST_LOCK.lock().await;
    let user_id = seed_test_user(&db).await;
    let group_a = seed_group(&db).await;
    let group_b = seed_group(&db).await;
    add_group_maintainer(&db, group_a, user_id).await;
    add_group_maintainer(&db, group_b, user_id).await;
    let file_id = insert_file_in_group(&db, group_a, "failed").await;
    let (task_id, item_id) =
        create_raw_active_file_task(&db, user_id, group_b, file_id, "queued").await;
    let lease_token = Uuid::new_v4();
    sqlx::query(
        "UPDATE context69.task_items SET status = 'running', lease_token = $2 WHERE id = $1",
    )
    .bind(item_id)
    .bind(lease_token)
    .execute(db.pool())
    .await
    .expect("lease foreign item");
    let attempt_id: i64 = sqlx::query_scalar(
        "INSERT INTO context69.task_attempts (task_id, item_id, attempt, status) \
         VALUES ($1, $2, 1, 'running') RETURNING id",
    )
    .bind(task_id)
    .bind(item_id)
    .fetch_one(db.pool())
    .await
    .expect("insert attempt");
    let updated = db
        .finish_task_item(context69::db::FinishTaskItemRequest {
            task_id,
            item_id,
            status: "succeeded",
            resource_id: None,
            failure_stage: None,
            error_message: None,
            retryable: false,
            lease_token,
            attempt_id,
        })
        .await
        .expect("finish item");
    assert!(updated);
    let (status, _, _) = file_status(&db, file_id).await;
    assert_eq!(
        status, "failed",
        "a foreign-group file must never be projected"
    );
    cleanup_task(&db, task_id).await;
    cleanup_file(&db, file_id).await;
    cleanup_group(&db, group_a).await;
    cleanup_group(&db, group_b).await;
    cleanup_user(&db, user_id).await;
}

#[tokio::test]
async fn missing_file_and_stale_status_are_noops() {
    let Some(db) = connect_scratch().await else {
        return;
    };
    let _guard = TEST_LOCK.lock().await;
    let user_id = seed_test_user(&db).await;
    let (file_id, group_id) = insert_file(&db, "failed").await;
    add_group_maintainer(&db, group_id, user_id).await;
    let (task_id, item_ids) =
        create_file_task(&db, user_id, group_id, file_id, "sql-opt-noop").await;
    let item_id = item_ids[0];

    // Stale status: the item is still queued, so projecting `succeeded` must
    // match no row.
    let stale = sqlx::query_file!(
        "src/sql/db/tasks/project_file_status.sql",
        item_id,
        "succeeded",
        Option::<String>::None
    )
    .execute(db.pool())
    .await
    .expect("stale projection")
    .rows_affected();
    assert_eq!(stale, 0, "stale status must update no file");

    // Missing item: a gone delete-batch row is a no-op.
    let missing = sqlx::query_file!(
        "src/sql/db/tasks/project_file_status.sql",
        Uuid::new_v4(),
        "succeeded",
        Option::<String>::None
    )
    .execute(db.pool())
    .await
    .expect("missing projection")
    .rows_affected();
    assert_eq!(missing, 0, "missing item must update no file");

    // NULL file: an item without a durable file is a no-op.
    let null_task = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO context69.tasks (id, user_id, group_id, kind, status, origin, \
         group_path, total_count, queued_count, stage) \
         VALUES ($1, $2, $3, 'file_batch', 'running', 'manual', 'test/sql-opt', 1, 1, 'storage')",
    )
    .bind(null_task)
    .bind(user_id)
    .bind(group_id)
    .execute(db.pool())
    .await
    .expect("insert null-file task");
    let null_item = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO context69.task_items (id, task_id, ordinal, payload, status, stage) \
         VALUES ($1, $2, 0, '{}', 'succeeded', 'storage')",
    )
    .bind(null_item)
    .bind(null_task)
    .execute(db.pool())
    .await
    .expect("insert null-file item");
    let null_hit = sqlx::query_file!(
        "src/sql/db/tasks/project_file_status.sql",
        null_item,
        "succeeded",
        Option::<String>::None
    )
    .execute(db.pool())
    .await
    .expect("null-file projection")
    .rows_affected();
    assert_eq!(null_hit, 0, "NULL file_id must update no file");

    let (status, _, _) = file_status(&db, file_id).await;
    assert_eq!(status, "failed");

    // Read the queued item row for the unused-variable lint.
    let row: Uuid = sqlx::query_scalar("SELECT id FROM context69.task_items WHERE id = $1")
        .bind(item_id)
        .fetch_one(db.pool())
        .await
        .expect("item still present");
    assert_eq!(row, item_id);

    sqlx::query("DELETE FROM context69.task_items WHERE task_id = $1")
        .bind(null_task)
        .execute(db.pool())
        .await
        .expect("clean null items");
    sqlx::query("DELETE FROM context69.tasks WHERE id = $1")
        .bind(null_task)
        .execute(db.pool())
        .await
        .expect("clean null task");
    cleanup_task(&db, task_id).await;
    cleanup_file(&db, file_id).await;
    cleanup_group(&db, group_id).await;
    cleanup_user(&db, user_id).await;
}
