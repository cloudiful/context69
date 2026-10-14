use crate::support::{LOCK, cleanup, connect, seed_task, seed_user};
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
        sqlx::query("UPDATE context69.task_items SET status = $2, waiting_reason = NULL, next_attempt_at = now() - interval '1 second', lease_until = now() - interval '1 second' WHERE id = $1")
            .bind(item).bind(status).execute(db.pool()).await.unwrap();
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
    assert!(
        !db.detach_unused_owned_docling_job(job.id, item, Uuid::new_v4())
            .await
            .unwrap()
    );
    assert!(
        !db.detach_unused_owned_docling_job(Uuid::new_v4(), item, owner.lease_token)
            .await
            .unwrap()
    );
    assert!(
        db.detach_unused_owned_docling_job(job.id, item, owner.lease_token)
            .await
            .unwrap()
    );
    assert!(
        !db.detach_unused_owned_docling_job(job.id, item, owner.lease_token)
            .await
            .unwrap()
    );
    assert!(
        db.get_active_docling_remote_job_for_item(item)
            .await
            .unwrap()
            .is_none()
    );
    let captured: (String, Option<String>, String) = sqlx::query_as("SELECT remote_task_id, endpoint_base_url, reason FROM context69.task_docling_remote_cleanup WHERE remote_job_id = $1")
        .bind(job.id).fetch_one(db.pool()).await.unwrap();
    assert_eq!(
        captured,
        (job.remote_task_id, job.endpoint_base_url, "orphan".into())
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
        !db.detach_unused_owned_docling_job(job.id, item, owner.lease_token)
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
    sqlx::query("DELETE FROM context69.task_docling_remote_cleanup WHERE remote_job_id = $1")
        .bind(job.id)
        .execute(db.pool())
        .await
        .unwrap();
    cleanup(&db, task, user).await;
}
