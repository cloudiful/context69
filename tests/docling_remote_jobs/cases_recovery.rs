use uuid::Uuid;

use crate::support::{
    LOCK, cleanup, connect, park_item_for_sweep, past_poll_at, seed_task, seed_user,
};

async fn seed_job(db: &context69::db::Database, task_id: Uuid, item_id: Uuid) -> Uuid {
    db.create_docling_remote_job(
        task_id,
        item_id,
        &format!("docling-{}", Uuid::new_v4()),
        Some("submitted"),
        Some(past_poll_at()),
        None,
    )
    .await
    .expect("remote job")
    .id
}

async fn active_row(
    db: &context69::db::Database,
    item_id: Uuid,
) -> Option<context69::db::StoredDoclingRemoteJob> {
    db.get_active_docling_remote_job_for_item(item_id)
        .await
        .expect("active lookup")
}

/// Terminal items fence their rows: a failed item's active reference is
/// cancelled and the item itself is left alone.
#[tokio::test]
async fn reconcile_cancels_rows_of_terminal_items() {
    let Some(db) = connect().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL missing; skipping");
        return;
    };
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id).await;
    seed_job(&db, task_id, item_id).await;
    sqlx::query(
        "UPDATE context69.task_items SET status = 'failed', lease_token = NULL, lease_until = NULL, \
         finished_at = now(), updated_at = now() WHERE id = $1",
    )
    .bind(item_id)
    .execute(db.pool())
    .await
    .expect("fail item");
    let summary = db
        .reconcile_docling_remote_state()
        .await
        .expect("reconcile");
    assert_eq!(summary.cancelled_remote_jobs, 1);
    assert_eq!(summary.adopted_items, 0);
    assert!(active_row(&db, item_id).await.is_none());
    cleanup(&db, task_id, user_id).await;
}

/// Bulk task cancel leaves remote rows behind (only the user cancel path
/// fences them atomically): recovery cancels rows whose task is terminal.
#[tokio::test]
async fn reconcile_cancels_rows_of_cancelled_tasks() {
    let Some(db) = connect().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL missing; skipping");
        return;
    };
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id).await;
    seed_job(&db, task_id, item_id).await;
    sqlx::query(
        "UPDATE context69.tasks SET status = 'cancelled', finished_at = now() WHERE id = $1",
    )
    .bind(task_id)
    .execute(db.pool())
    .await
    .expect("cancel task");
    sqlx::query(
        "UPDATE context69.task_items SET status = 'cancelled', lease_token = NULL, lease_until = NULL, \
         finished_at = now() WHERE task_id = $1",
    )
    .bind(task_id)
    .execute(db.pool())
    .await
    .expect("cancel items");
    let summary = db
        .reconcile_docling_remote_state()
        .await
        .expect("reconcile");
    assert_eq!(summary.cancelled_remote_jobs, 1);
    assert!(active_row(&db, item_id).await.is_none());
    cleanup(&db, task_id, user_id).await;
}

/// A crashed worker (item still `running` but its lease expired) keeps its
/// remote reference: recovery never touches running rows, so the reclaim
/// adopts the tracked remote id instead of submitting a fresh conversion.
#[tokio::test]
async fn reconcile_preserves_expired_running_rows_for_adoption() {
    let Some(db) = connect().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL missing; skipping");
        return;
    };
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id).await;
    let job_id = seed_job(&db, task_id, item_id).await;
    sqlx::query(
        "UPDATE context69.task_items SET status = 'running', lease_token = $2, \
         lease_until = now() - interval '1 minute' WHERE id = $1",
    )
    .bind(item_id)
    .bind(Uuid::new_v4())
    .execute(db.pool())
    .await
    .expect("crashed worker state");
    let summary = db
        .reconcile_docling_remote_state()
        .await
        .expect("reconcile");
    assert_eq!(summary.cancelled_remote_jobs, 0);
    assert_eq!(summary.adopted_items, 0);
    assert_eq!(
        active_row(&db, item_id).await.expect("row kept").id,
        job_id,
        "recovery must not cancel the row a reclaimed worker adopts"
    );
    use sqlx::Row;
    let status: String = sqlx::query("SELECT status FROM context69.task_items WHERE id = $1")
        .bind(item_id)
        .fetch_one(db.pool())
        .await
        .expect("read item")
        .get("status");
    assert_eq!(status, "running", "recovery never moves the item itself");
    db.cancel_active_docling_remote_job_for_item(item_id, Some("test done"))
        .await
        .expect("cancel remote");
    cleanup(&db, task_id, user_id).await;
}

/// Crash adoption end to end at the claim boundary: after the worker lease
/// expires, the dispatcher reclaims the same item while its remote row stays
/// active, so the resumed worker polls the existing remote id (one remote id
/// per conversion, no duplicate submit).
#[tokio::test]
async fn expired_lease_reclaim_adopts_the_same_remote_id() {
    let Some(db) = connect().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL missing; skipping");
        return;
    };
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id).await;
    let job_id = seed_job(&db, task_id, item_id).await;
    let dead_lease = Uuid::new_v4();
    sqlx::query(
        "UPDATE context69.task_items SET status = 'running', lease_token = $2, \
         lease_until = now() - interval '1 minute' WHERE id = $1",
    )
    .bind(item_id)
    .bind(dead_lease)
    .execute(db.pool())
    .await
    .expect("crashed worker state");
    // Recovery runs first (as it does every 30s in production) and must leave
    // the row alone; the claim then reclaims the item with the row intact.
    let summary = db
        .reconcile_docling_remote_state()
        .await
        .expect("reconcile");
    assert_eq!(summary.cancelled_remote_jobs, 0);
    let claimed = db
        .claim_items_fast(50)
        .await
        .expect("fast claim")
        .into_iter()
        .find(|row| row.id == item_id)
        .expect("expired running item with an active row is reclaimable");
    assert_ne!(
        claimed.lease_token, dead_lease,
        "reclaim mints a fresh item lease"
    );
    assert_eq!(
        active_row(&db, item_id).await.expect("row kept").id,
        job_id,
        "the same remote id survives the worker lease expiry"
    );
    db.cancel_active_docling_remote_job_for_item(item_id, Some("test done"))
        .await
        .expect("cancel remote");
    cleanup(&db, task_id, user_id).await;
}

/// A live worker (running item, live lease) owns its row: recovery changes
/// nothing, so the blocking poll loop is never fenced mid-conversion.
#[tokio::test]
async fn reconcile_leaves_live_workers_alone() {
    let Some(db) = connect().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL missing; skipping");
        return;
    };
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id).await;
    let job_id = seed_job(&db, task_id, item_id).await;
    sqlx::query(
        "UPDATE context69.task_items SET status = 'running', lease_token = $2, \
         lease_until = now() + interval '5 minutes' WHERE id = $1",
    )
    .bind(item_id)
    .bind(Uuid::new_v4())
    .execute(db.pool())
    .await
    .expect("live worker state");
    let summary = db
        .reconcile_docling_remote_state()
        .await
        .expect("reconcile");
    assert_eq!(summary.cancelled_remote_jobs, 0);
    assert_eq!(summary.adopted_items, 0);
    assert_eq!(active_row(&db, item_id).await.expect("row kept").id, job_id);
    db.cancel_active_docling_remote_job_for_item(item_id, Some("test done"))
        .await
        .expect("cancel remote");
    cleanup(&db, task_id, user_id).await;
}

/// A `queued` item with an active row (adoption transient) and a backoff
/// park with an active row (worker parked after its inline budget) both
/// resolve by cancelling the row: the item becomes claimable and resumes
/// with a fresh submit.
#[tokio::test]
async fn reconcile_cancels_rows_without_a_live_owner() {
    let Some(db) = connect().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL missing; skipping");
        return;
    };
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (queued_task, queued_item) = seed_task(&db, user_id).await;
    seed_job(&db, queued_task, queued_item).await;
    let (backoff_task, backoff_item) = seed_task(&db, user_id).await;
    seed_job(&db, backoff_task, backoff_item).await;
    sqlx::query(
        "UPDATE context69.task_items SET status = 'waiting', waiting_reason = 'backoff', \
         next_attempt_at = now() + interval '1 minute' WHERE id = $1",
    )
    .bind(backoff_item)
    .execute(db.pool())
    .await
    .expect("backoff park");
    let summary = db
        .reconcile_docling_remote_state()
        .await
        .expect("reconcile");
    assert_eq!(summary.cancelled_remote_jobs, 2);
    assert_eq!(summary.adopted_items, 0);
    assert!(active_row(&db, queued_item).await.is_none());
    assert!(active_row(&db, backoff_item).await.is_none());
    cleanup(&db, queued_task, user_id).await;
    cleanup(&db, backoff_task, user_id).await;
}

/// Legacy adoption preserves the parked payload: the requeued item carries
/// the same content and becomes due immediately.
#[tokio::test]
async fn reconcile_adoption_preserves_payload_and_marks_due() {
    let Some(db) = connect().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL missing; skipping");
        return;
    };
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id).await;
    seed_job(&db, task_id, item_id).await;
    park_item_for_sweep(&db, item_id).await;
    let summary = db
        .reconcile_docling_remote_state()
        .await
        .expect("reconcile");
    assert_eq!(summary.adopted_items, 1);
    use sqlx::Row;
    let row = sqlx::query(
        "SELECT status, waiting_reason, payload, next_attempt_at \
         FROM context69.task_items WHERE id = $1",
    )
    .bind(item_id)
    .fetch_one(db.pool())
    .await
    .expect("read item");
    let status: String = row.get("status");
    let reason: Option<String> = row.get("waiting_reason");
    let payload: serde_json::Value = row.get("payload");
    let due: chrono::DateTime<chrono::Utc> = row.get("next_attempt_at");
    assert_eq!(status, "queued");
    assert_eq!(reason, None);
    assert_eq!(payload, serde_json::json!({"external_id": "a"}));
    assert!(due <= chrono::Utc::now());
    cleanup(&db, task_id, user_id).await;
}
