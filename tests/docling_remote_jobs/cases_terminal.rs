use chrono::Duration;
use uuid::Uuid;

use crate::support::{LOCK, cleanup, connect, past_poll_at, seed_task, seed_user};

#[tokio::test]
async fn terminal_finish_blocks_stale_writes() {
    let Some(db) = connect().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL missing; skipping");
        return;
    };
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id).await;
    let remote_id = format!("docling-{}", Uuid::new_v4());
    let job = db
        .create_docling_remote_job(
            task_id,
            item_id,
            &remote_id,
            None,
            Some(past_poll_at()),
            None,
        )
        .await
        .expect("create");
    let claimed = db
        .claim_due_docling_remote_jobs(10, 30)
        .await
        .expect("claim");
    let lease = claimed
        .iter()
        .find(|row| row.id == job.id)
        .expect("claimed")
        .lease_token
        .expect("lease");
    let finished = db
        .finish_docling_remote_job(job.id, lease, "succeeded", Some("done"), None)
        .await
        .expect("finish")
        .expect("succeeds once");
    assert_eq!(finished.status, "succeeded");
    assert!(finished.finished_at.is_some());
    assert!(
        db.finish_docling_remote_job(job.id, lease, "failed", Some("done"), Some("late"))
            .await
            .expect("dup")
            .is_none(),
        "late duplicate changes nothing"
    );
    assert!(
        db.get_active_docling_remote_job_for_item(item_id)
            .await
            .expect("active")
            .is_none(),
        "terminal leaves no active row"
    );
    db.create_docling_remote_job(
        task_id,
        item_id,
        &format!("docling-{}", Uuid::new_v4()),
        None,
        None,
        None,
    )
    .await
    .expect("new active after terminal");
    cleanup(&db, task_id, user_id).await;
}

#[tokio::test]
async fn cancel_and_deadline_timeout_paths() {
    let Some(db) = connect().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL missing; skipping");
        return;
    };
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (cancel_task, cancel_item) = seed_task(&db, user_id).await;
    let (expire_task, expire_item) = seed_task(&db, user_id).await;
    let cancel_job = db
        .create_docling_remote_job(
            cancel_task,
            cancel_item,
            &format!("docling-{}", Uuid::new_v4()),
            None,
            Some(past_poll_at()),
            None,
        )
        .await
        .expect("cancellable");
    let claimed = db
        .claim_due_docling_remote_jobs(10, 30)
        .await
        .expect("claim");
    let lease = claimed
        .iter()
        .find(|row| row.id == cancel_job.id)
        .expect("claimed")
        .lease_token
        .expect("lease");
    let cancelled = db
        .cancel_active_docling_remote_job_for_item(cancel_item, Some("owning item cancelled"))
        .await
        .expect("cancel")
        .expect("cancelled without lease");
    assert_eq!(cancelled.status, "cancelled");
    assert!(
        db.finish_docling_remote_job(cancel_job.id, lease, "succeeded", Some("done"), None)
            .await
            .expect("late")
            .is_none(),
        "cancel revokes in-flight lease"
    );
    db.create_docling_remote_job(
        expire_task,
        expire_item,
        &format!("docling-{}", Uuid::new_v4()),
        Some("pending"),
        Some(chrono::Utc::now() - Duration::seconds(60)),
        Some(chrono::Utc::now() - Duration::seconds(1)),
    )
    .await
    .expect("expired");
    let timed_out = db
        .timeout_expired_docling_remote_jobs(10, Some("deadline exceeded"))
        .await
        .expect("timeout");
    let row = timed_out
        .iter()
        .find(|row| row.item_id == expire_item)
        .expect("expired returned");
    assert_eq!(row.status, "timed_out");
    cleanup(&db, cancel_task, user_id).await;
    cleanup(&db, expire_task, user_id).await;
}
