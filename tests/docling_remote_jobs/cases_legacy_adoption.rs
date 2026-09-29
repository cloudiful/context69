use chrono::Duration;
use uuid::Uuid;

use crate::support::{
    LOCK, cleanup, connect, park_item_for_sweep, past_poll_at, seed_task, seed_user,
};

async fn item_state(
    db: &context69::db::Database,
    item_id: Uuid,
) -> (String, Option<String>, serde_json::Value) {
    use sqlx::Row;

    let row = sqlx::query(
        "SELECT status, waiting_reason, payload FROM context69.task_items WHERE id = $1",
    )
    .bind(item_id)
    .fetch_one(db.pool())
    .await
    .expect("read item");
    (
        row.get("status"),
        row.get("waiting_reason"),
        row.get("payload"),
    )
}

/// Dispatcher exclusion for legacy parks: an item with an active remote job
/// is never claimed, even when its waiting backoff is due. The row stays out
/// of the dispatcher until recovery adopts the item; the claim keeps no
/// parked item away from normal workers once the row is gone.
#[tokio::test]
async fn dispatcher_excludes_legacy_park_until_recovery_adopts_it() {
    let Some(db) = connect().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL missing; skipping");
        return;
    };
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id).await;
    db.create_docling_remote_job(
        task_id,
        item_id,
        &format!("docling-{}", Uuid::new_v4()),
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
    assert!(
        !db.claim_items_fast(10)
            .await
            .expect("fast claim")
            .iter()
            .any(|row| row.id == item_id),
        "dispatcher must not claim an item with an active remote job"
    );
    let summary = db
        .reconcile_docling_remote_state()
        .await
        .expect("reconcile");
    assert_eq!(summary.adopted_items, 1, "the legacy park is adopted");
    assert_eq!(summary.cancelled_remote_jobs, 1);
    let (status, reason, _) = item_state(&db, item_id).await;
    assert_eq!(status, "queued");
    assert_eq!(reason, None);
    assert!(
        db.get_active_docling_remote_job_for_item(item_id)
            .await
            .expect("active")
            .is_none(),
        "adoption cancels the orphaned row so the next claim adopts the item"
    );
    cleanup(&db, task_id, user_id).await;
}

/// A `waiting/docling` item is never claimable by the dispatcher, even when
/// its remote row already reads terminal and its backoff is due. Only
/// recovery moves such items.
#[tokio::test]
async fn dispatcher_never_claims_waiting_docling_items() {
    let Some(db) = connect().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL missing; skipping");
        return;
    };
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id).await;
    db.create_docling_remote_job(
        task_id,
        item_id,
        &format!("docling-{}", Uuid::new_v4()),
        Some("pending"),
        Some(past_poll_at()),
        Some(chrono::Utc::now() - Duration::seconds(1)),
    )
    .await
    .expect("expired job");
    db.cancel_active_docling_remote_job_for_item(item_id, Some("test done"))
        .await
        .expect("terminal row");
    park_item_for_sweep(&db, item_id).await;
    sqlx::query("UPDATE context69.task_items SET next_attempt_at = now() - interval '1 second' WHERE id = $1")
        .bind(item_id)
        .execute(db.pool())
        .await
        .expect("make waiting due");
    assert!(
        !db.claim_items_fast(10)
            .await
            .expect("fast claim")
            .iter()
            .any(|row| row.id == item_id),
        "dispatcher must not resubmit a parked item, terminal remote or not"
    );
    db.cancel_active_docling_remote_job_for_item(item_id, Some("test done"))
        .await
        .expect("cancel remote");
    cleanup(&db, task_id, user_id).await;
}

/// Recovery never polls the remote: a due active row keeps its observed
/// remote state, attempt count, and due time. There is no background
/// poll queue left; only the owning worker advances conversions.
#[tokio::test]
async fn recovery_never_polls_the_remote() {
    let Some(db) = connect().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL missing; skipping");
        return;
    };
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id).await;
    let due = past_poll_at();
    let created = db
        .create_docling_remote_job(
            task_id,
            item_id,
            &format!("docling-{}", Uuid::new_v4()),
            Some("submitted"),
            Some(due),
            None,
        )
        .await
        .expect("due job");
    park_item_for_sweep(&db, item_id).await;
    // Adopt the item first so the remaining active row below belongs to a
    // live running worker, which recovery must leave alone.
    let lease = Uuid::new_v4();
    sqlx::query(
        "UPDATE context69.task_items SET status = 'running', lease_token = $2, \
         lease_until = now() + interval '5 minutes' WHERE id = $1",
    )
    .bind(item_id)
    .bind(lease)
    .execute(db.pool())
    .await
    .expect("worker holds the item");
    db.reconcile_docling_remote_state()
        .await
        .expect("reconcile");
    let row = db
        .get_active_docling_remote_job_for_item(item_id)
        .await
        .expect("active")
        .expect("live worker rows are untouched");
    assert_eq!(row.id, created.id);
    assert_eq!(row.remote_status.as_deref(), Some("submitted"));
    assert_eq!(row.attempt_count, 0);
    assert_eq!(row.next_poll_at, created.next_poll_at);
    db.cancel_active_docling_remote_job_for_item(item_id, Some("test done"))
        .await
        .expect("cancel remote");
    cleanup(&db, task_id, user_id).await;
}
