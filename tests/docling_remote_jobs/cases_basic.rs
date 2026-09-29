use chrono::{Duration, Utc};
use uuid::Uuid;

use crate::support::{LOCK, cleanup, connect, seed_task, seed_user};

#[tokio::test]
async fn create_and_fetch_roundtrip() {
    let Some(db) = connect().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL missing; skipping");
        return;
    };
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id).await;
    let remote_id = format!("docling-{}", Uuid::new_v4());
    let created = db
        .create_docling_remote_job(
            task_id,
            item_id,
            &remote_id,
            Some("queued"),
            Some(Utc::now() - Duration::seconds(1)),
            Some(Utc::now() + Duration::hours(1)),
        )
        .await
        .expect("create");
    assert_eq!(created.provider, "docling");
    assert_eq!(created.status, "pending");
    assert_eq!(created.attempt_count, 0);
    assert!(created.lease_token.is_none());
    let by_item = db
        .get_active_docling_remote_job_for_item(item_id)
        .await
        .expect("get item")
        .expect("active exists");
    assert_eq!(by_item.remote_task_id, remote_id);
    cleanup(&db, task_id, user_id).await;
}

#[tokio::test]
async fn rejects_second_active_and_duplicate_remote_id() {
    let Some(db) = connect().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL missing; skipping");
        return;
    };
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id).await;
    let (other_task, other_item) = seed_task(&db, user_id).await;
    let remote_id = format!("docling-{}", Uuid::new_v4());
    db.create_docling_remote_job(task_id, item_id, &remote_id, None, None, None)
        .await
        .expect("first");
    let second = db
        .create_docling_remote_job(
            task_id,
            item_id,
            &format!("docling-{}", Uuid::new_v4()),
            None,
            None,
            None,
        )
        .await;
    assert!(second.is_err(), "one active job per item");
    let dup = db
        .create_docling_remote_job(other_task, other_item, &remote_id, None, None, None)
        .await;
    assert!(dup.is_err(), "remote_task_id is unique");
    cleanup(&db, task_id, user_id).await;
    cleanup(&db, other_task, user_id).await;
}
