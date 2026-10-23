use crate::support::{
    CleanupRowSeed, LOCK, cleanup, connect, delete_cleanup_rows, seed_cleanup_row, seed_task,
    seed_user,
};
use chrono::{DateTime, Duration, Timelike, Utc};
use uuid::Uuid;

/// PostgreSQL timestamps persist microseconds, so fixtures bound to that
/// precision keep exact persisted-time assertions rather than comparing the
/// host clock's nanoseconds with the truncated stored value.
fn postgres_precision(value: DateTime<Utc>) -> DateTime<Utc> {
    value
        .with_nanosecond(value.nanosecond() / 1_000 * 1_000)
        .expect("microsecond-aligned UTC timestamp")
}

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
    // The 60s lease is database-defined: claim_due.sql sets
    // claim_until = now() + 60s and updated_at = now() in the same statement,
    // so both timestamps are compared on the database clock, never the host.
    assert!(row.cleanup_deadline_at < row.updated_at);
    assert_eq!(
        row.claim_until,
        Some(row.updated_at + chrono::Duration::seconds(60))
    );
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

/// Seeds rows far in the future so paging assertions are unaffected by rows
/// other suites left in the shared cleanup outbox; deltas keep counts exact.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn cleanup_page_reports_true_total_reaches_beyond_the_cap_and_filters_exactly() {
    let db = connect().await;
    let _guard = LOCK.lock().await;
    let user = seed_user(&db).await;
    let (task, item) = seed_task(&db, user).await;

    let before_total = db.count_docling_cleanup(None, None, None).await.unwrap();
    let before_done_success = db
        .count_docling_cleanup(Some("done"), Some("success"), None)
        .await
        .unwrap();
    let before_done_any = db
        .count_docling_cleanup(Some("done"), None, None)
        .await
        .unwrap();
    let before_skipped = db
        .count_docling_cleanup(Some("done"), Some("skipped"), None)
        .await
        .unwrap();
    let before_pending = db
        .count_docling_cleanup(Some("pending"), None, None)
        .await
        .unwrap();

    let base = postgres_precision(Utc::now() + Duration::days(3650));
    for index in 0..120 {
        let created = base + Duration::seconds(index);
        seed_cleanup_row(
            &db,
            task,
            item,
            CleanupRowSeed {
                remote_task_id: &format!("paging-bulk-{index:03}"),
                status: "done",
                outcome: Some("success"),
                created_at: created,
                updated_at: created,
                finished_at: Some(created),
            },
        )
        .await;
    }

    let older = postgres_precision(base - Duration::days(1));
    seed_cleanup_row(
        &db,
        task,
        item,
        CleanupRowSeed {
            remote_task_id: "paging-pending-null",
            status: "pending",
            outcome: None,
            created_at: older,
            updated_at: older,
            finished_at: None,
        },
    )
    .await;
    seed_cleanup_row(
        &db,
        task,
        item,
        CleanupRowSeed {
            remote_task_id: "paging-skipped",
            status: "done",
            outcome: Some("skipped"),
            created_at: older,
            updated_at: older,
            finished_at: Some(older),
        },
    )
    .await;
    seed_cleanup_row(
        &db,
        task,
        item,
        CleanupRowSeed {
            remote_task_id: "paging-partial",
            status: "failed",
            outcome: Some("partial_success"),
            created_at: older,
            updated_at: older,
            finished_at: Some(older),
        },
    )
    .await;

    let timed_created = postgres_precision(older - Duration::hours(2));
    let timed_updated = timed_created + Duration::minutes(5);
    let timed_finished = timed_updated + Duration::seconds(1);
    seed_cleanup_row(
        &db,
        task,
        item,
        CleanupRowSeed {
            remote_task_id: "paging-timed",
            status: "done",
            outcome: None,
            created_at: timed_created,
            updated_at: timed_updated,
            finished_at: Some(timed_finished),
        },
    )
    .await;

    let after_total = db.count_docling_cleanup(None, None, None).await.unwrap();
    assert_eq!(after_total - before_total, 124);
    assert_eq!(
        db.count_docling_cleanup(Some("done"), Some("success"), None)
            .await
            .unwrap()
            - before_done_success,
        120
    );
    assert_eq!(
        db.count_docling_cleanup(Some("done"), None, None)
            .await
            .unwrap()
            - before_done_any,
        122,
        "an absent outcome filter keeps the null-outcome row"
    );
    assert_eq!(
        db.count_docling_cleanup(Some("done"), Some("skipped"), None)
            .await
            .unwrap()
            - before_skipped,
        1
    );
    assert_eq!(
        db.count_docling_cleanup(Some("pending"), None, None)
            .await
            .unwrap()
            - before_pending,
        1
    );
    assert_eq!(
        db.count_docling_cleanup(None, None, Some("paging-bulk-000"))
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        db.count_docling_cleanup(None, None, Some("paging-absent"))
            .await
            .unwrap(),
        0
    );
    assert!(
        db.list_docling_cleanup_page(None, None, Some("paging-absent"), 20, 0)
            .await
            .unwrap()
            .is_empty(),
        "an exact filter with no match is an empty page, not an error"
    );

    let page_six = db
        .list_docling_cleanup_page(None, None, None, 20, 100)
        .await
        .unwrap();
    assert_eq!(
        page_six.len(),
        20,
        "page 6 at size 20 must reach past the legacy 100-row window"
    );
    assert_eq!(page_six[0].remote_task_id, "paging-bulk-019");
    assert_eq!(page_six[19].remote_task_id, "paging-bulk-000");

    let timed = db
        .list_docling_cleanup_page(None, None, Some("paging-timed"), 20, 0)
        .await
        .unwrap();
    assert_eq!(timed.len(), 1);
    assert_eq!(timed[0].created_at, timed_created);
    assert_eq!(timed[0].updated_at, timed_updated);
    assert_eq!(timed[0].finished_at, Some(timed_finished));

    let tie_at = postgres_precision(base + Duration::seconds(10_000));
    for remote in ["paging-tie-a", "paging-tie-b", "paging-tie-c"] {
        seed_cleanup_row(
            &db,
            task,
            item,
            CleanupRowSeed {
                remote_task_id: remote,
                status: "pending",
                outcome: None,
                created_at: tie_at,
                updated_at: tie_at,
                finished_at: None,
            },
        )
        .await;
    }
    let mut tie_rows = Vec::new();
    for page in 1..=3 {
        let rows = db
            .list_docling_cleanup_page(None, None, None, 1, i64::from(page - 1))
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        tie_rows.push(rows[0].clone());
    }
    let tie_ids = tie_rows.iter().map(|row| row.id).collect::<Vec<_>>();
    let sorted_ids = {
        let mut sorted = tie_ids.clone();
        sorted.sort_by(|left, right| right.cmp(left));
        sorted
    };
    assert_eq!(tie_ids, sorted_ids, "equal created_at ties descend by id");
    for remote in ["paging-tie-a", "paging-tie-b", "paging-tie-c"] {
        assert!(
            tie_rows.iter().any(|row| row.remote_task_id == remote),
            "the tied page must contain {remote}"
        );
    }

    delete_cleanup_rows(&db, task).await;
    cleanup(&db, task, user).await;
}
