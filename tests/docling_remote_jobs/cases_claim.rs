use chrono::Duration;
use uuid::Uuid;

use crate::support::{LOCK, cleanup, connect, past_poll_at, seed_task, seed_user};

/// Due-row claiming is scoped by id, never by global counts: the shared
/// scratch DB may hold leftover due rows from other suites, so every
/// assertion filters to the rows this test created.
#[tokio::test]
async fn claim_lease_pending_and_reclaim_after_expiry() {
    let Some(db) = connect().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL missing; skipping");
        return;
    };
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id).await;
    let (future_task, future_item) = seed_task(&db, user_id).await;
    let remote_id = format!("docling-{}", Uuid::new_v4());
    let future_remote = format!("docling-{}", Uuid::new_v4());
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
        .expect("due job");
    db.create_docling_remote_job(
        future_task,
        future_item,
        &future_remote,
        None,
        Some(chrono::Utc::now() + Duration::hours(1)),
        None,
    )
    .await
    .expect("future job");
    let claimed = db
        .claim_due_docling_remote_jobs(10, 30)
        .await
        .expect("claim");
    assert!(
        claimed.iter().any(|row| row.id == job.id),
        "due job is claimed"
    );
    assert!(
        !claimed
            .iter()
            .any(|row| row.remote_task_id == future_remote),
        "future job is not claimed"
    );
    let lease = claimed
        .iter()
        .find(|row| row.id == job.id)
        .expect("due claimed")
        .lease_token
        .expect("lease installed");
    let again = db
        .claim_due_docling_remote_jobs(10, 30)
        .await
        .expect("reclaim");
    assert!(
        !again.iter().any(|row| row.id == job.id),
        "held lease blocks a second claim of the same row"
    );
    let hb = db
        .heartbeat_docling_remote_job(job.id, lease, 60)
        .await
        .expect("hb")
        .expect("holder extends");
    assert_eq!(hb.lease_token, Some(lease));
    assert!(
        db.heartbeat_docling_remote_job(job.id, Uuid::new_v4(), 60)
            .await
            .expect("stale hb")
            .is_none(),
        "wrong token extends nothing"
    );
    let next_poll = chrono::Utc::now() + Duration::seconds(120);
    let pending = db
        .record_docling_remote_job_pending(job.id, lease, Some("pending"), next_poll, None)
        .await
        .expect("record")
        .expect("holder records");
    assert_eq!(pending.status, "running");
    assert_eq!(pending.attempt_count, 1);
    assert!(pending.lease_token.is_none(), "poll releases lease");
    assert!(
        db.record_docling_remote_job_pending(job.id, lease, Some("pending"), next_poll, None)
            .await
            .expect("stale")
            .is_none(),
        "released lease cannot write twice"
    );
    sqlx::query("UPDATE context69.task_docling_remote_jobs SET next_poll_at = now() - interval '1 second', lease_token = NULL, lease_until = NULL WHERE id = $1")
        .bind(job.id).execute(db.pool()).await.expect("make due");
    let reclaimed = db
        .claim_due_docling_remote_jobs(10, 30)
        .await
        .expect("reclaim");
    let fresh = reclaimed
        .iter()
        .find(|row| row.id == job.id)
        .expect("restart recovery reclaims the due row");
    assert_ne!(fresh.lease_token, Some(lease), "fresh lease minted");
    cleanup(&db, task_id, user_id).await;
    cleanup(&db, future_task, user_id).await;
}

#[tokio::test]
async fn expired_deadline_is_never_claimed() {
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
            Some("pending"),
            Some(past_poll_at()),
            Some(chrono::Utc::now() - Duration::seconds(1)),
        )
        .await
        .expect("expired job");
    let claimed = db
        .claim_due_docling_remote_jobs(10, 30)
        .await
        .expect("claim");
    assert!(
        !claimed.iter().any(|row| row.id == job.id),
        "expired deadline is filtered from due claims"
    );
    let timed_out = db
        .timeout_expired_docling_remote_jobs(10, Some("deadline exceeded"))
        .await
        .expect("timeout");
    assert!(
        timed_out.iter().any(|row| row.id == job.id),
        "expired job is timed out instead"
    );
    cleanup(&db, task_id, user_id).await;
}
