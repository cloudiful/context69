//! Trash/restore lifecycle, active-task rejection, and ownership.

use context69::db::Database;
use sqlx::Row;

use super::support::{create_task, finish_task, item_count, seed_test_user, test_database_url};

#[tokio::test]
async fn trash_and_restore_round_trip_is_idempotent() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping trash round-trip test");
        return;
    };
    let _guard = super::support::TEST_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let (task_id, _items) = create_task(&db, user_id, "round-trip").await;
    finish_task(&db, task_id, "succeeded").await;

    let active = db
        .list_tasks(context69::db::TaskListFilter {
            user_id,
            query: None,
            kind: None,
            status: None,
            stage: None,
            waiting_reason: None,
            dependency_key: None,
            sort_by: None,
            sort_direction: None,
            limit: 50,
            offset: 0,
            view: "completed",
        })
        .await
        .expect("list active tasks");
    assert!(active.iter().any(|task| task.id == task_id));
    let trashed = db
        .list_tasks(context69::db::TaskListFilter {
            user_id,
            query: None,
            kind: None,
            status: None,
            stage: None,
            waiting_reason: None,
            dependency_key: None,
            sort_by: None,
            sort_direction: None,
            limit: 50,
            offset: 0,
            view: "trash",
        })
        .await
        .expect("list trashed tasks");
    assert!(!trashed.iter().any(|task| task.id == task_id));

    assert!(db.trash_task(task_id).await.expect("trash task"));
    let stored = db
        .get_task_internal(task_id)
        .await
        .expect("load task")
        .expect("task exists");
    assert!(stored.deleted_at.is_some(), "trash must set deleted_at");
    assert_eq!(stored.status, "succeeded");

    // A repeat trash is a no-op: the row is already trashed.
    assert!(!db.trash_task(task_id).await.expect("repeat trash"));

    let active = db
        .list_tasks(context69::db::TaskListFilter {
            user_id,
            query: None,
            kind: None,
            status: None,
            stage: None,
            waiting_reason: None,
            dependency_key: None,
            sort_by: None,
            sort_direction: None,
            limit: 50,
            offset: 0,
            view: "completed",
        })
        .await
        .expect("list active tasks");
    assert!(!active.iter().any(|task| task.id == task_id));
    let trashed = db
        .list_tasks(context69::db::TaskListFilter {
            user_id,
            query: None,
            kind: None,
            status: None,
            stage: None,
            waiting_reason: None,
            dependency_key: None,
            sort_by: None,
            sort_direction: None,
            limit: 50,
            offset: 0,
            view: "trash",
        })
        .await
        .expect("list trashed tasks");
    assert!(trashed.iter().any(|task| task.id == task_id));

    assert!(db.restore_task(task_id).await.expect("restore task"));
    let stored = db
        .get_task_internal(task_id)
        .await
        .expect("load task")
        .expect("task exists");
    assert!(stored.deleted_at.is_none(), "restore must clear deleted_at");
    // A repeat restore is a no-op.
    assert!(!db.restore_task(task_id).await.expect("repeat restore"));

    let active = db
        .list_tasks(context69::db::TaskListFilter {
            user_id,
            query: None,
            kind: None,
            status: None,
            stage: None,
            waiting_reason: None,
            dependency_key: None,
            sort_by: None,
            sort_direction: None,
            limit: 50,
            offset: 0,
            view: "completed",
        })
        .await
        .expect("list active tasks");
    assert!(active.iter().any(|task| task.id == task_id));
}

#[tokio::test]
async fn trashing_preserves_items_and_rejects_active_tasks() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping trash preserve test");
        return;
    };
    let _guard = super::support::TEST_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;

    let (active_task, _items) = create_task(&db, user_id, "active").await;
    assert!(
        !db.trash_task(active_task).await.expect("active trash"),
        "active task must not be trashed"
    );
    let stored = db
        .get_task_internal(active_task)
        .await
        .expect("load task")
        .expect("task exists");
    assert!(stored.deleted_at.is_none());

    let (terminal_task, _items) = create_task(&db, user_id, "terminal").await;
    finish_task(&db, terminal_task, "succeeded").await;
    let before = item_count(&db, terminal_task).await;

    assert!(db.trash_task(terminal_task).await.expect("trash task"));
    assert_eq!(
        item_count(&db, terminal_task).await,
        before,
        "trashing must not delete task items"
    );
    let item_status: String =
        sqlx::query("SELECT status FROM context69.task_items WHERE task_id = $1 LIMIT 1")
            .bind(terminal_task)
            .fetch_one(db.pool())
            .await
            .expect("load item")
            .get("status");
    assert_eq!(
        item_status, "succeeded",
        "trashing must not change item status"
    );
}

#[tokio::test]
async fn trash_actions_respect_task_ownership() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping trash ownership test");
        return;
    };
    let _guard = super::support::TEST_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let owner_id = seed_test_user(&db).await;
    let stranger_id = seed_test_user(&db).await;
    let (task_id, _items) = create_task(&db, owner_id, "owned").await;
    finish_task(&db, task_id, "succeeded").await;

    assert!(
        db.can_manage_task(task_id, owner_id)
            .await
            .expect("owner access")
    );
    assert!(
        !db.can_manage_task(task_id, stranger_id)
            .await
            .expect("stranger access")
    );
    assert!(
        db.get_task(task_id, stranger_id)
            .await
            .expect("stranger visibility")
            .is_none(),
        "a task must not be visible to an unrelated user"
    );
}
