use chrono::Duration;
use serde_json::json;
use uuid::Uuid;

use crate::support::{
    LOCK, cleanup, connect, park_item_for_sweep, past_poll_at, seed_task, seed_user,
};

/// Dispatcher exclusion: an item with an active remote job is never claimed,
/// even when its waiting backoff is due. The sweep owns it until terminal.
#[tokio::test]
async fn dispatcher_skips_items_with_active_remote_jobs() {
    let Some(db) = connect().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL missing; skipping");
        return;
    };
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id).await;
    let remote_id = format!("docling-{}", Uuid::new_v4());
    db.create_docling_remote_job(
        task_id,
        item_id,
        &remote_id,
        None,
        Some(past_poll_at()),
        None,
    )
    .await
    .expect("remote job");
    park_item_for_sweep(&db, item_id).await;
    sqlx::query("UPDATE context69.task_items SET next_attempt_at = now() - interval '1 second' WHERE id = $1")
        .bind(item_id)
        .execute(db.pool())
        .await
        .expect("make waiting due");
    let claimed = db.claim_items_fast(10).await.expect("fast claim");
    assert!(
        !claimed.iter().any(|row| row.id == item_id),
        "dispatcher must not claim an item with an active remote job"
    );
    db.cancel_active_docling_remote_job_for_item(item_id, Some("test done"))
        .await
        .expect("cancel remote");
    cleanup(&db, task_id, user_id).await;
}

/// Timeout sweep fails the parked item atomically: one call moves the remote
/// row and the item together, so no crash window can strand a terminal row
/// next to a still-parked item.
#[tokio::test]
async fn sweep_timeout_fails_the_parked_item() {
    let Some(db) = connect().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL missing; skipping");
        return;
    };
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id).await;
    let remote_id = format!("docling-{}", Uuid::new_v4());
    db.create_docling_remote_job(
        task_id,
        item_id,
        &remote_id,
        Some("pending"),
        Some(past_poll_at()),
        Some(chrono::Utc::now() - Duration::seconds(1)),
    )
    .await
    .expect("expired job");
    park_item_for_sweep(&db, item_id).await;
    let expired = db
        .claim_expired_docling_remote_jobs(10)
        .await
        .expect("expiry claim");
    let job = expired
        .iter()
        .find(|row| row.item_id == item_id)
        .expect("expired job is discovered without being marked terminal");
    assert_eq!(job.status, "pending");
    let finished = db
        .fail_expired_docling_remote_job(job, "deadline exceeded")
        .await
        .expect("atomic timeout")
        .expect("remote and item move together");
    assert_eq!(finished.status, "timed_out");
    let item = db
        .list_task_items(task_id, 10, 0)
        .await
        .expect("list items")
        .into_iter()
        .find(|row| row.id == item_id)
        .expect("item exists");
    assert_eq!(item.status, "failed");
    assert!(
        db.get_active_docling_remote_job_for_item(item_id)
            .await
            .expect("active")
            .is_none()
    );
    cleanup(&db, task_id, user_id).await;
}

/// Success finalize requeues the parked item with the fetched sections under
/// the shared `section_payload` key so downstream stages resume at
/// `embedding` without re-entering Docling.
#[tokio::test]
async fn sweep_success_requeues_with_sections_payload() {
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
        .expect("remote job");
    park_item_for_sweep(&db, item_id).await;
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
    let sections = json!([{"section_key": "document", "body_text": "hello"}]);
    let payload = json!({"external_id": "a", "section_payload": sections.clone()});
    assert_eq!(payload.get("section_payload"), Some(&sections));
    let finished = db
        .finish_docling_remote_job_with_requeue(
            job.id,
            lease,
            Some("success"),
            item_id,
            task_id,
            &payload,
        )
        .await
        .expect("requeue")
        .expect("fences pass");
    assert_eq!(finished.status, "succeeded");
    assert!(
        db.claim_items_fast(10)
            .await
            .expect("claim after requeue")
            .iter()
            .any(|row| row.id == item_id),
        "requeued item is claimable once its remote job is terminal"
    );
    cleanup(&db, task_id, user_id).await;
}

/// Sweep observability counts move together: an active due job is due but
/// not in-flight until claimed, then in-flight until its outcome is recorded.
#[tokio::test]
async fn sweep_counts_track_due_and_inflight() {
    let Some(db) = connect().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL missing; skipping");
        return;
    };
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id).await;
    let before = db.docling_remote_job_counts().await.expect("counts");
    db.create_docling_remote_job(
        task_id,
        item_id,
        &format!("docling-{}", Uuid::new_v4()),
        None,
        Some(past_poll_at()),
        None,
    )
    .await
    .expect("due job");
    let during = db.docling_remote_job_counts().await.expect("counts");
    assert!(during.active_count > before.active_count);
    assert!(during.due_count > before.due_count);
    cleanup(&db, task_id, user_id).await;
}
