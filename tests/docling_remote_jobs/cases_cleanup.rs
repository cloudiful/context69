use crate::support::{LOCK, cleanup, connect, seed_task, seed_user};
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn terminal_recovery_captures_deadline_and_failure_but_not_new_attempt() {
    let db = connect().await;
    let _guard = LOCK.lock().await;
    for expired in [true, false] {
        let user = seed_user(&db).await;
        let (task, item) = seed_task(&db, user).await;
        let deadline = if expired {
            chrono::Utc::now() - chrono::Duration::seconds(1)
        } else {
            chrono::Utc::now() + chrono::Duration::hours(1)
        };
        let job = db
            .create_docling_remote_job(
                task,
                item,
                &Uuid::new_v4().to_string(),
                None,
                None,
                Some(deadline),
                Some("http://snapshot.invalid/v1"),
            )
            .await
            .unwrap();
        sqlx::query("UPDATE context69.task_items SET status = 'failed' WHERE id = $1")
            .bind(item)
            .execute(db.pool())
            .await
            .unwrap();
        db.reconcile_docling_remote_state().await.unwrap();
        let captured: (String, String, Option<String>) = sqlx::query_as("SELECT reason, remote_task_id, endpoint_base_url FROM context69.task_docling_remote_cleanup WHERE remote_job_id = $1").bind(job.id).fetch_one(db.pool()).await.unwrap();
        assert_eq!(
            captured,
            (
                (if expired {
                    "deadline"
                } else {
                    "permanent_failure"
                })
                .into(),
                job.remote_task_id,
                job.endpoint_base_url
            )
        );
        sqlx::query("UPDATE context69.task_items SET status = 'queued' WHERE id = $1")
            .bind(item)
            .execute(db.pool())
            .await
            .unwrap();
        let fresh = db
            .create_docling_remote_job(
                task,
                item,
                &Uuid::new_v4().to_string(),
                None,
                None,
                None,
                Some("http://new-setting.invalid/v1"),
            )
            .await
            .unwrap();
        db.reconcile_docling_remote_state().await.unwrap();
        assert_eq!(
            db.get_active_docling_remote_job_for_item(item)
                .await
                .unwrap()
                .unwrap()
                .id,
            fresh.id
        );
        sqlx::query("DELETE FROM context69.task_docling_remote_cleanup WHERE remote_job_id = $1")
            .bind(job.id)
            .execute(db.pool())
            .await
            .unwrap();
        cleanup(&db, task, user).await;
    }
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn restart_claim_recovers_expired_lease_and_deadline_rows() {
    let db = connect().await;
    let _guard = LOCK.lock().await;
    let user = seed_user(&db).await;
    let (task, item) = seed_task(&db, user).await;
    let job = Uuid::new_v4();
    db.enqueue_direct_docling_cleanup(job, task, item, &Uuid::new_v4().to_string(), None, "orphan")
        .await
        .unwrap();
    let old = Uuid::new_v4();
    sqlx::query("UPDATE context69.task_docling_remote_cleanup SET next_attempt_at = '1990-01-01', cleanup_deadline_at = now() - interval '1 second', claim_token = $2, claim_until = now() - interval '1 second' WHERE remote_job_id = $1").bind(job).bind(old).execute(db.pool()).await.unwrap();
    let current = Uuid::new_v4();
    let claimed = db.claim_docling_cleanup(16, current).await.unwrap();
    assert!(claimed.len() <= 16);
    let row = claimed
        .into_iter()
        .find(|row| row.remote_job_id == job)
        .unwrap();
    assert_eq!(row.claim_token, Some(current));
    assert!(row.cleanup_deadline_at < chrono::Utc::now());
    assert!(row.claim_until.unwrap() <= chrono::Utc::now() + chrono::Duration::seconds(60));
    assert!(
        !db.settle_docling_cleanup(row.id, "done", None, None, chrono::Utc::now(), old)
            .await
            .unwrap()
    );
    assert!(
        db.settle_docling_cleanup(
            row.id,
            "failed",
            None,
            Some("Remote termination unconfirmed"),
            chrono::Utc::now(),
            current
        )
        .await
        .unwrap()
    );
    sqlx::query("DELETE FROM context69.task_docling_remote_cleanup WHERE remote_job_id = $1")
        .bind(job)
        .execute(db.pool())
        .await
        .unwrap();
    cleanup(&db, task, user).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn cancellation_cleanup_is_atomic_idempotent_and_survives_purge() {
    let db = connect().await;
    let _guard = LOCK.lock().await;
    let user = seed_user(&db).await;
    let (task, item) = seed_task(&db, user).await;
    let remote = Uuid::new_v4().to_string();
    let job = db
        .create_docling_remote_job(
            task,
            item,
            &remote,
            None,
            None,
            None,
            Some("http://snapshot.invalid"),
        )
        .await
        .unwrap();
    let mut tx = db.pool().begin().await.unwrap();
    sqlx::query(include_str!("../../src/sql/db/tasks/cancel.sql"))
        .bind(task)
        .bind(user)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query(include_str!(
        "../../src/sql/db/tasks/docling_remote_jobs/enqueue_cleanup_for_task.sql"
    ))
    .bind(task)
    .bind("business_cancel")
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::query(include_str!(
        "../../src/sql/db/tasks/docling_remote_jobs/cancel_active_for_task.sql"
    ))
    .bind(task)
    .bind(Option::<&str>::None)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(
        db.get_active_docling_remote_job_for_item(item)
            .await
            .unwrap()
            .unwrap()
            .id,
        job.id
    );
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM context69.task_docling_remote_cleanup WHERE remote_job_id = $1",
    )
    .bind(job.id)
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(count, 0);
    assert!(db.cancel_task(task, user).await.unwrap());
    assert!(!db.cancel_task(task, user).await.unwrap());
    let identities: Vec<(String, Option<String>)> = sqlx::query_as("SELECT remote_task_id, endpoint_base_url FROM context69.task_docling_remote_cleanup WHERE remote_job_id = $1").bind(job.id).fetch_all(db.pool()).await.unwrap();
    assert_eq!(
        identities,
        vec![(remote, Some("http://snapshot.invalid".into()))]
    );
    assert!(db.trash_task(task).await.unwrap());
    assert!(db.delete_trashed_task(task).await.unwrap());
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM context69.task_docling_remote_cleanup WHERE remote_job_id = $1",
    )
    .bind(job.id)
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(count, 1);
    sqlx::query("DELETE FROM context69.task_docling_remote_cleanup WHERE remote_job_id = $1")
        .bind(job.id)
        .execute(db.pool())
        .await
        .unwrap();
    cleanup(&db, task, user).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn claims_are_exclusive_and_stale_settle_cannot_overwrite() {
    let db = connect().await;
    let _guard = LOCK.lock().await;
    let user = seed_user(&db).await;
    let (task, item) = seed_task(&db, user).await;
    let job = Uuid::new_v4();
    db.enqueue_direct_docling_cleanup(job, task, item, &Uuid::new_v4().to_string(), None, "orphan")
        .await
        .unwrap();
    sqlx::query("UPDATE context69.task_docling_remote_cleanup SET next_attempt_at = '2000-01-01' WHERE remote_job_id = $1").bind(job).execute(db.pool()).await.unwrap();
    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    let (first, second) = tokio::join!(
        db.claim_docling_cleanup(1, a),
        db.claim_docling_cleanup(1, b)
    );
    let rows = first
        .unwrap()
        .into_iter()
        .chain(second.unwrap())
        .filter(|row| row.remote_job_id == job)
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert!(
        !db.settle_docling_cleanup(
            row.id,
            "done",
            Some("cancelled"),
            None,
            chrono::Utc::now(),
            Uuid::new_v4()
        )
        .await
        .unwrap()
    );
    assert!(
        db.settle_docling_cleanup(
            row.id,
            "unsupported",
            None,
            Some("Submit endpoint unavailable"),
            chrono::Utc::now(),
            row.claim_token.unwrap()
        )
        .await
        .unwrap()
    );
    assert!(
        !db.settle_docling_cleanup(
            row.id,
            "done",
            Some("cancelled"),
            None,
            chrono::Utc::now(),
            row.claim_token.unwrap()
        )
        .await
        .unwrap()
    );
    sqlx::query("DELETE FROM context69.task_docling_remote_cleanup WHERE remote_job_id = $1")
        .bind(job)
        .execute(db.pool())
        .await
        .unwrap();
    cleanup(&db, task, user).await;
}
