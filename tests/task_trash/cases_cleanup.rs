//! Retention cleanup isolation and permanent-delete eligibility.

use chrono::{Duration, Utc};
use context69::db::Database;
use sqlx::Row;

use super::support::{create_task, finish_task, seed_test_user, test_database_url};

#[tokio::test]
async fn retention_cleanup_only_purges_trashed_terminal_tasks() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping cleanup isolation test");
        return;
    };
    let _guard = super::support::TEST_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;

    let (kept_task, _items) = create_task(&db, user_id, "kept").await;
    finish_task(&db, kept_task, "succeeded").await;
    let (trashed_task, trashed_items) = create_task(&db, user_id, "trashed").await;
    finish_task(&db, trashed_task, "failed").await;
    assert!(db.trash_task(trashed_task).await.expect("trash task"));

    let cutoff = Utc::now() + Duration::seconds(60);
    let deleted = db
        .cleanup_expired_terminal_tasks(cutoff, 100)
        .await
        .expect("cleanup expired");
    assert!(
        deleted.contains(&trashed_task),
        "trashed terminal task must be purged"
    );
    assert!(
        !deleted.contains(&kept_task),
        "non-trashed terminal task must survive automatic cleanup"
    );
    assert!(
        db.get_task_internal(trashed_task)
            .await
            .expect("load trashed")
            .is_none(),
        "trashed task row must be deleted"
    );
    assert!(
        db.get_task_internal(kept_task)
            .await
            .expect("load kept")
            .is_some(),
        "active-history task row must survive"
    );
    for item_id in trashed_items {
        let exists: bool =
            sqlx::query("SELECT EXISTS (SELECT 1 FROM context69.task_items WHERE id = $1) AS e")
                .bind(item_id)
                .fetch_one(db.pool())
                .await
                .expect("item existence")
                .get("e");
        assert!(!exists, "task items must cascade with the deleted task");
    }

    // The explicit admin all-terminal purge keeps its legacy semantics: it
    // removes terminal history regardless of the trash marker.
    let purged = db.purge_terminal_tasks(100).await.expect("purge terminal");
    assert!(purged.contains(&kept_task));
}

#[tokio::test]
async fn permanent_delete_requires_a_trashed_task() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping permanent delete test");
        return;
    };
    let _guard = super::support::TEST_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
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
