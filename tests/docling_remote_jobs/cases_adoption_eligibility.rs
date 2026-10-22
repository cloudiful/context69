use crate::support::{LOCK, claim_item, cleanup, connect, crash_holder, seed_task, seed_user};
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn due_remote_owners_are_claimed_once_with_identity_retained() {
    let db = connect().await;
    let _guard = LOCK.lock().await;
    for status in ["queued", "waiting", "running"] {
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
                Some("http://snapshot.invalid"),
            )
            .await
            .unwrap();
        if status == "running" {
            // A running owner is a claimed item whose worker died: the
            // authoritative library attempt and its domain mirror expire
            // together, so the claim may take the item over.
            let holder = claim_item(&db, item).await;
            crash_holder(&db, &holder).await;
        } else {
            sqlx::query_file!(
                "tests/sql/task_execution/release_item_to_due.sql",
                item,
                status
            )
            .execute(db.pool())
            .await
            .unwrap();
        }
        let (first, second) = tokio::join!(db.claim_items_fast(10), db.claim_items_fast(10));
        let claims = first
            .unwrap()
            .into_iter()
            .chain(second.unwrap())
            .filter(|row| row.id == item)
            .collect::<Vec<_>>();
        assert_eq!(claims.len(), 1, "one fresh owner for {status}");
        let adopted = db
            .get_active_docling_remote_job_for_item(item)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(adopted.id, job.id);
        assert_eq!(adopted.remote_task_id, job.remote_task_id);
        assert_eq!(adopted.endpoint_base_url, job.endpoint_base_url);
        cleanup(&db, task, user).await;
    }
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn unused_detach_is_exact_job_and_live_lease_fenced() {
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
            Some("http://snapshot.invalid"),
        )
        .await
        .unwrap();
    let owner = db
        .claim_items_fast(10)
        .await
        .unwrap()
        .into_iter()
        .find(|row| row.id == item)
        .unwrap();
    let execution = owner
        .execution_context()
        .expect("the claimed item carries its captured execution");
    // The captured execution of an earlier generation belongs to a segment the
    // item does not hold any more, so the detach is refused.
    let stale = context69::db::ExecutionContext {
        attempt: execution.attempt.clone(),
        generation: execution.generation + 1,
        item_id: execution.item_id,
    };
    assert!(
        !db.detach_unused_owned_docling_job(job.id, item, &stale)
            .await
            .unwrap()
    );
    assert!(
        !db.detach_unused_owned_docling_job(Uuid::new_v4(), item, &execution)
            .await
            .unwrap()
    );
    assert!(
        db.detach_unused_owned_docling_job(job.id, item, &execution)
            .await
            .unwrap()
    );
    assert!(
        !db.detach_unused_owned_docling_job(job.id, item, &execution)
            .await
            .unwrap()
    );
    assert!(
        db.get_active_docling_remote_job_for_item(item)
            .await
            .unwrap()
            .is_none()
    );
    let captured = sqlx::query_file!(
        "tests/sql/task_execution/read_docling_cleanup_row.sql",
        job.id
    )
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(
        (
            captured.remote_task_id,
            captured.endpoint_base_url,
            captured.reason
        ),
        (
            job.remote_task_id,
            job.endpoint_base_url,
            "orphan".to_string()
        )
    );
    let replacement = db
        .create_docling_remote_job(
            task,
            item,
            &Uuid::new_v4().to_string(),
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
    assert!(
        !db.detach_unused_owned_docling_job(job.id, item, &execution)
            .await
            .unwrap()
    );
    assert_eq!(
        db.get_active_docling_remote_job_for_item(item)
            .await
            .unwrap()
            .unwrap()
            .id,
        replacement.id
    );
    sqlx::query_file!(
        "tests/sql/task_execution/delete_docling_cleanup_row.sql",
        job.id
    )
    .execute(db.pool())
    .await
    .unwrap();
    cleanup(&db, task, user).await;
}
