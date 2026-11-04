//! Durable Docling remote-job references for the bounded probe flow
//! (issue #650 P3, issue 1172 P4).
//!
//! The remote row is the known identity a claimed segment probes at most once:
//! a waiting observation parks the item on the job's persisted due time and
//! releases the library attempt, and the recovery pass only fences terminal
//! owners. Its ordinary park cadence belongs to the dispatcher, not to the
//! periodic recovery tick.
//! Split per reviewer note 9599: shared fixtures live in `support`, each case
//! file stays cohesive and under 300 lines. All assertions are scoped by id,
//! never by global counts, so leftover rows in the shared scratch DB cannot
//! flake the suite.

#[cfg(feature = "integration-test-helpers")]
#[path = "replacement_runner/qdrant_points_mock.rs"]
mod qdrant_points_mock;

#[cfg(feature = "integration-test-helpers")]
#[path = "task_source_rerun/support.rs"]
mod source_rerun_support;

#[path = "docling_remote_jobs/support.rs"]
mod support;

#[path = "docling_remote_jobs/cases_basic.rs"]
mod cases_basic;

#[path = "docling_remote_jobs/cases_terminal.rs"]
mod cases_terminal;

#[path = "docling_remote_jobs/cases_legacy_adoption.rs"]
mod cases_legacy_adoption;

#[path = "docling_remote_jobs/cases_recovery.rs"]
mod cases_recovery;

#[path = "docling_remote_jobs/cases_adoption_eligibility.rs"]
mod cases_adoption_eligibility;

#[path = "docling_remote_jobs/cases_cleanup.rs"]
mod cases_cleanup;

#[path = "docling_remote_jobs/cases_park_probe.rs"]
mod cases_park_probe;

#[cfg(feature = "integration-test-helpers")]
#[path = "docling_remote_jobs/cases_uncertainty.rs"]
mod cases_uncertainty;

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn adopted_identity_blocks_stale_claim_and_admin_wake() {
    use support::{LOCK, cleanup, connect, seed_task, seed_user};
    use uuid::Uuid;
    let db = connect().await;
    let _guard = LOCK.lock().await;
    let user = seed_user(&db).await;
    let (task, item) = seed_task(&db, user).await;
    let job = db
        .create_docling_remote_job(
            task,
            item,
            &Uuid::new_v4().to_string(),
            None,
            None,
            None,
            Some("http://original.invalid/v1"),
        )
        .await
        .unwrap();
    assert!(
        !db.enqueue_direct_docling_cleanup(
            Uuid::new_v4(),
            task,
            item,
            job.remote_task_id.as_deref().expect("known remote id"),
            job.endpoint_base_url.as_deref(),
            "orphan"
        )
        .await
        .unwrap()
    );
    let token = Uuid::new_v4();
    let id: Uuid = sqlx::query_scalar("INSERT INTO context69.task_docling_remote_cleanup (remote_job_id, task_id, item_id, remote_task_id, endpoint_base_url, claim_token, claim_until, next_attempt_at) VALUES ($1,$2,$3,$4,$5,$6,now() + interval '1 minute',now() + interval '1 hour') RETURNING id")
        .bind(job.id).bind(task).bind(item).bind(job.remote_task_id.as_deref()).bind(&job.endpoint_base_url).bind(token).fetch_one(db.pool()).await.unwrap();
    assert!(
        !db.docling_cleanup_claim_is_detached(id, token)
            .await
            .unwrap()
    );
    assert!(!db.wake_due_docling_cleanup().await.unwrap().contains(&id));
    assert_eq!(
        db.get_active_docling_remote_job_for_item(item)
            .await
            .unwrap()
            .unwrap()
            .id,
        job.id
    );
    db.cancel_task(task, user).await.unwrap();
    assert!(
        db.docling_cleanup_claim_is_detached(id, token)
            .await
            .unwrap()
    );
    assert!(
        !db.docling_cleanup_claim_is_detached(id, Uuid::new_v4())
            .await
            .unwrap()
    );
    sqlx::query("UPDATE context69.task_docling_remote_cleanup SET claim_until = now() - interval '1 second' WHERE id = $1").bind(id).execute(db.pool()).await.unwrap();
    assert!(db.wake_due_docling_cleanup().await.unwrap().contains(&id));
    assert!(
        !db.docling_cleanup_claim_is_detached(id, token)
            .await
            .unwrap()
    );
    sqlx::query("DELETE FROM context69.task_docling_remote_cleanup WHERE id = $1")
        .bind(id)
        .execute(db.pool())
        .await
        .unwrap();
    cleanup(&db, task, user).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn bulk_cancel_captures_owned_snapshot_before_fencing() {
    use support::{LOCK, cleanup, connect, seed_task, seed_user};
    use uuid::Uuid;
    let db = connect().await;
    let _guard = LOCK.lock().await;
    let user = seed_user(&db).await;
    let (task, item) = seed_task(&db, user).await;
    let job = db
        .create_docling_remote_job(
            task,
            item,
            &Uuid::new_v4().to_string(),
            None,
            None,
            None,
            Some("http://bulk-snapshot.invalid/v1"),
        )
        .await
        .unwrap();
    assert!(db.cancel_all_active_tasks().await.unwrap() >= 1);
    assert!(
        db.get_active_docling_remote_job_for_item(item)
            .await
            .unwrap()
            .is_none()
    );
    let row: (String, Option<String>, String) = sqlx::query_as("SELECT remote_task_id, endpoint_base_url, reason FROM context69.task_docling_remote_cleanup WHERE remote_job_id = $1").bind(job.id).fetch_one(db.pool()).await.unwrap();
    assert_eq!(
        row,
        (
            job.remote_task_id.clone().expect("known remote id"),
            job.endpoint_base_url,
            "business_cancel".into()
        )
    );
    sqlx::query("DELETE FROM context69.task_docling_remote_cleanup WHERE remote_job_id = $1")
        .bind(job.id)
        .execute(db.pool())
        .await
        .unwrap();
    cleanup(&db, task, user).await;
}
