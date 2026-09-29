use serde_json::json;
use uuid::Uuid;

use crate::support::{LOCK, cleanup, connect, past_poll_at, seed_task, seed_user};

/// Puts the item in `running` under `lease`, mirroring a dispatcher claim
/// without touching any other suite's rows on the shared scratch DB.
async fn hold_item(db: &context69::db::Database, item_id: Uuid, lease: Uuid) {
    sqlx::query(
        "UPDATE context69.task_items SET status = 'running', lease_token = $2, \
         lease_until = now() + interval '5 minutes', attempt_count = 1 WHERE id = $1",
    )
    .bind(item_id)
    .bind(lease)
    .execute(db.pool())
    .await
    .expect("hold item");
}

async fn item_payload(db: &context69::db::Database, item_id: Uuid) -> serde_json::Value {
    use sqlx::Row;

    sqlx::query("SELECT payload FROM context69.task_items WHERE id = $1")
        .bind(item_id)
        .fetch_one(db.pool())
        .await
        .expect("read payload")
        .get("payload")
}

/// The atomic inline success commit marks the remote row `succeeded` and
/// persists the fetched sections into the running item's payload together:
/// no crash window can strand a terminal row next to an item without
/// sections, or sections next to an active row that would resubmit.
#[tokio::test]
async fn finish_with_sections_commits_success_atomically() {
    let Some(db) = connect().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL missing; skipping");
        return;
    };
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id).await;
    let lease = Uuid::new_v4();
    hold_item(&db, item_id, lease).await;
    let job = db
        .create_docling_remote_job(
            task_id,
            item_id,
            &format!("docling-{}", Uuid::new_v4()),
            Some("submitted"),
            Some(past_poll_at()),
            None,
        )
        .await
        .expect("remote job");
    let sections = json!([{"section_key": "document", "body_text": "hello"}]);
    let payload = json!({"external_id": "a", "section_payload": sections.clone()});
    let committed = db
        .finish_docling_remote_job_with_sections(job.id, item_id, lease, &payload, "success")
        .await
        .expect("commit")
        .expect("fences pass");
    assert_eq!(committed, item_id);
    assert_eq!(item_payload(&db, item_id).await, payload);
    assert!(
        db.get_active_docling_remote_job_for_item(item_id)
            .await
            .expect("active")
            .is_none(),
        "committed row leaves no active reference"
    );
    cleanup(&db, task_id, user_id).await;
}

/// The commit is fenced on both sides: a stale item lease, an already
/// terminal row, and a duplicate commit all change nothing.
#[tokio::test]
async fn finish_with_sections_is_fenced_and_idempotent() {
    let Some(db) = connect().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL missing; skipping");
        return;
    };
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id).await;
    let lease = Uuid::new_v4();
    hold_item(&db, item_id, lease).await;
    let job = db
        .create_docling_remote_job(
            task_id,
            item_id,
            &format!("docling-{}", Uuid::new_v4()),
            None,
            Some(past_poll_at()),
            None,
        )
        .await
        .expect("remote job");
    let payload = json!({"external_id": "a", "section_payload": []});
    assert!(
        db.finish_docling_remote_job_with_sections(
            job.id,
            item_id,
            Uuid::new_v4(),
            &payload,
            "success"
        )
        .await
        .expect("stale lease")
        .is_none(),
        "a worker that lost its lease commits nothing"
    );
    assert!(
        db.get_active_docling_remote_job_for_item(item_id)
            .await
            .expect("active")
            .is_some(),
        "fenced commit leaves the row active for the rightful owner"
    );
    assert_eq!(
        item_payload(&db, item_id).await,
        json!({"external_id": "a"}),
        "fenced commit leaves the payload alone"
    );
    db.cancel_active_docling_remote_job_for_item(item_id, Some("test done"))
        .await
        .expect("cancel row");
    assert!(
        db.finish_docling_remote_job_with_sections(job.id, item_id, lease, &payload, "success")
            .await
            .expect("cancelled row")
            .is_none(),
        "a terminal row is never resurrected"
    );
    cleanup(&db, task_id, user_id).await;
}

/// Cancel fences the row without a lease, and a late success commit after
/// the cancel changes nothing; a fresh submit is allowed once the row is
/// terminal.
#[tokio::test]
async fn cancel_and_late_commit_paths() {
    let Some(db) = connect().await else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL missing; skipping");
        return;
    };
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id).await;
    let lease = Uuid::new_v4();
    hold_item(&db, item_id, lease).await;
    let job = db
        .create_docling_remote_job(
            task_id,
            item_id,
            &format!("docling-{}", Uuid::new_v4()),
            None,
            Some(past_poll_at()),
            None,
        )
        .await
        .expect("cancellable");
    let cancelled = db
        .cancel_active_docling_remote_job_for_item(item_id, Some("owning item cancelled"))
        .await
        .expect("cancel")
        .expect("cancelled without lease");
    assert_eq!(cancelled.status, "cancelled");
    assert!(
        db.finish_docling_remote_job_with_sections(
            job.id,
            item_id,
            lease,
            &json!({"section_payload": []}),
            "success"
        )
        .await
        .expect("late")
        .is_none(),
        "cancel wins over a late success commit"
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
