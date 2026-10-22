use serde_json::json;
use uuid::Uuid;

use crate::support::{LOCK, cleanup, connect, past_poll_at, seed_task, seed_user};

/// The captured execution of an earlier generation: the item belongs to a
/// newer segment, so the transition must be refused.
fn stale_execution(execution: &context69::db::ExecutionContext) -> context69::db::ExecutionContext {
    context69::db::ExecutionContext {
        attempt: execution.attempt.clone(),
        generation: execution.generation + 1,
        item_id: execution.item_id,
    }
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
#[ignore = "requires configured reusable PostgreSQL"]
async fn finish_with_sections_commits_success_atomically() {
    let db = connect().await;
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id).await;
    let holder = crate::support::claim_item(&db, item_id).await;
    let execution = holder
        .execution_context()
        .expect("the claimed item carries its captured execution");
    let job = db
        .create_docling_remote_job(
            task_id,
            item_id,
            &format!("docling-{}", Uuid::new_v4()),
            Some("submitted"),
            Some(past_poll_at()),
            None,
            None,
        )
        .await
        .expect("remote job");
    let sections = json!([{"section_key": "document", "body_text": "hello"}]);
    let payload = json!({"external_id": "a", "section_payload": sections.clone()});
    let committed = db
        .finish_docling_remote_job_with_sections(job.id, item_id, &execution, &payload, "success")
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
#[ignore = "requires configured reusable PostgreSQL"]
async fn finish_with_sections_is_fenced_and_idempotent() {
    let db = connect().await;
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id).await;
    let holder = crate::support::claim_item(&db, item_id).await;
    let execution = holder
        .execution_context()
        .expect("the claimed item carries its captured execution");
    let job = db
        .create_docling_remote_job(
            task_id,
            item_id,
            &format!("docling-{}", Uuid::new_v4()),
            None,
            Some(past_poll_at()),
            None,
            None,
        )
        .await
        .expect("remote job");
    let payload = json!({"external_id": "a", "section_payload": []});
    assert!(
        db.finish_docling_remote_job_with_sections(
            job.id,
            item_id,
            &stale_execution(&execution),
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
        db.finish_docling_remote_job_with_sections(
            job.id, item_id, &execution, &payload, "success",
        )
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
#[ignore = "requires configured reusable PostgreSQL"]
async fn cancel_and_late_commit_paths() {
    let db = connect().await;
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id).await;
    let holder = crate::support::claim_item(&db, item_id).await;
    let execution = holder
        .execution_context()
        .expect("the claimed item carries its captured execution");
    let job = db
        .create_docling_remote_job(
            task_id,
            item_id,
            &format!("docling-{}", Uuid::new_v4()),
            None,
            Some(past_poll_at()),
            None,
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
            &execution,
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
        None,
    )
    .await
    .expect("new active after terminal");
    cleanup(&db, task_id, user_id).await;
}

/// A crashed worker cannot commit the conversion: the unified transaction rolls
/// the remote job, the payload and the section checkpoint back together.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_crashed_worker_commits_no_conversion_output() {
    let db = connect().await;
    let _guard = LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let (task_id, item_id) = seed_task(&db, user_id).await;
    let holder = crate::support::claim_item(&db, item_id).await;
    let execution = holder
        .execution_context()
        .expect("the claimed item carries its captured execution");
    let job = db
        .create_docling_remote_job(
            task_id,
            item_id,
            &format!("docling-{}", Uuid::new_v4()),
            Some("submitted"),
            Some(past_poll_at()),
            None,
            None,
        )
        .await
        .expect("remote job");
    let before = item_payload(&db, item_id).await;

    crate::support::crash_holder(&db, &holder).await;

    let payload = json!({"external_id": "a", "section_payload": [{"section_key": "document"}]});
    assert!(
        db.finish_docling_remote_job_with_sections(
            job.id, item_id, &execution, &payload, "success",
        )
        .await
        .expect("a refused commit is not an error")
        .is_none(),
        "the expired segment commits no conversion output"
    );
    assert!(
        db.get_active_docling_remote_job_for_item(item_id)
            .await
            .expect("active")
            .is_some(),
        "the remote job stays active for the owner that inherits it"
    );
    assert_eq!(
        item_payload(&db, item_id).await,
        before,
        "no section checkpoint was committed"
    );

    cleanup(&db, task_id, user_id).await;
}
