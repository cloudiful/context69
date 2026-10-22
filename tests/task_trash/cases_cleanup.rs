//! Permanent-delete eligibility (issue 391 Task 1: automatic retention
//! cleanup and admin terminal purge removed; only per-task trash/restore/
//! permanent delete remains).

use context69::db::Database;

use super::support::{create_task, finish_task, seed_test_user};

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn permanent_delete_requires_a_trashed_task() {
    let url = context69_test_support::accessor::database_url();
    let _guard = super::support::TEST_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    context69_db_schema::bootstrap_library_schema(db.pool())
        .await
        .expect("bootstrap the library schema");
    let user_id = seed_test_user(&db).await;
    let (task_id, _items) = create_task(&db, user_id, "delete").await;
    finish_task(&db, task_id, "cancelled").await;

    assert!(
        !db.delete_trashed_task(task_id)
            .await
            .expect("delete active-history task"),
        "a non-trashed task must not be permanently deleted"
    );
    assert!(
        db.get_task_internal(task_id)
            .await
            .expect("load task")
            .is_some()
    );

    assert!(db.trash_task(task_id).await.expect("trash task"));
    assert!(
        db.delete_trashed_task(task_id)
            .await
            .expect("delete trashed task")
    );
    assert!(
        db.get_task_internal(task_id)
            .await
            .expect("load deleted task")
            .is_none()
    );
}
