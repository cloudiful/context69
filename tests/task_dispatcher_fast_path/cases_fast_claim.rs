use context69::db::Database;
use serde_json::json;
use uuid::Uuid;

use crate::support::{
    FAST_PATH_LOCK, cleanup_task, cleanup_user, seed_test_user, test_database_url,
};

async fn insert_docling_external_job(
    db: &Database,
    item_id: Uuid,
    status: &str,
    next_poll_at: chrono::DateTime<chrono::Utc>,
    deadline_at: chrono::DateTime<chrono::Utc>,
) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO context69.task_external_jobs \
         (id, item_id, provider, remote_task_id, status, submitted_at, next_poll_at, deadline_at, submission_count) \
         VALUES ($1, $2, 'docling', $3, $4, now(), $5, $6, 1)",
    )
    .bind(id)
    .bind(item_id)
    .bind(format!("remote-{id}"))
    .bind(status)
    .bind(next_poll_at)
    .bind(deadline_at)
    .execute(db.pool())
    .await
    .expect("insert docling external job");
    id
}

#[tokio::test]
async fn claim_items_fast_does_not_starve_ordinary_tasks_when_polls_are_due() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping poll fairness test");
        return;
    };
    let _guard = FAST_PATH_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;

    // Ordinary queued item first so FIFO age puts it before the polls. With
    // the old poll-first ordering a limit-2 claim would return only polls.
    let ordinary_task = Uuid::new_v4();
    let (ordinary_task, _reused, ordinary_items) = db
        .create_task_submission(
            ordinary_task,
            user_id,
            None,
            "text_batch",
            Some("test/fast-path"),
            None,
            &[json!({"external_id": "fair-ordinary"})],
            None,
            &format!("fast-path-fair-ordinary-{}", Uuid::new_v4()),
        )
        .await
        .expect("create ordinary task");

    let poll_task = Uuid::new_v4();
    let (poll_task, _reused, poll_items) = db
        .create_task_submission(
            poll_task,
            user_id,
            None,
            "text_batch",
            Some("test/fast-path"),
            None,
            &[
                json!({"external_id": "fair-poll-a"}),
                json!({"external_id": "fair-poll-b"}),
            ],
            None,
            &format!("fast-path-fair-poll-{}", Uuid::new_v4()),
        )
        .await
        .expect("create poll task");
    for item_id in &poll_items {
        sqlx::query(
            "UPDATE context69.task_items \
             SET status = 'waiting', stage = 'docling_poll', waiting_reason = 'external_job', \
                 attempt_count = 6, next_attempt_at = now() - interval '1 minute', waiting_since = now() \
             WHERE id = $1",
        )
        .bind(*item_id)
        .execute(db.pool())
        .await
        .expect("park poll item");
    }
    sqlx::query(
        "UPDATE context69.tasks SET status = 'waiting', stage = 'docling_poll', waiting_reason = 'external_job', \
             next_attempt_at = now() - interval '1 minute' WHERE id = $1",
    )
    .bind(poll_task)
    .execute(db.pool())
    .await
    .expect("park poll task");
    let deadline = chrono::Utc::now() + chrono::Duration::hours(1);
    let next_poll = chrono::Utc::now() - chrono::Duration::seconds(30);
    let mut poll_jobs = Vec::new();
    for item_id in &poll_items {
        poll_jobs
            .push(insert_docling_external_job(&db, *item_id, "running", next_poll, deadline).await);
    }

    let claimed = db.claim_items_fast(2).await.expect("fair claim");
    let claimed_ordinary = claimed.iter().any(|item| item.task_id == ordinary_task);
    let claimed_poll = claimed.iter().any(|item| item.task_id == poll_task);
    assert!(
        claimed_ordinary && claimed_poll,
        "limit-2 claim with 1 ordinary + 2 due polls must interleave (got {} ordinary, {} poll in {:?}); polls must not monopolize the batch",
        claimed_ordinary as u8,
        claimed_poll as u8,
        claimed
            .iter()
            .map(|item| (item.task_id, item.id))
            .collect::<Vec<_>>(),
    );
    assert_eq!(claimed.len(), 2);

    for job_id in poll_jobs {
        sqlx::query("DELETE FROM context69.task_external_jobs WHERE id = $1")
            .bind(job_id)
            .execute(db.pool())
            .await
            .expect("clean up poll job");
    }
    for item_id in ordinary_items.iter().chain(poll_items.iter()) {
        sqlx::query("DELETE FROM context69.task_attempts WHERE item_id = $1")
            .bind(*item_id)
            .execute(db.pool())
            .await
            .expect("clean up attempts");
    }
    cleanup_task(&db, poll_task, user_id).await;
    cleanup_task(&db, ordinary_task, user_id).await;
    let _ = ordinary_items;
    cleanup_user(&db, user_id).await;
}

#[tokio::test]
async fn claim_items_fast_claims_and_activates_a_fresh_task() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping fast claim activation test");
        return;
    };
    let _guard = FAST_PATH_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;
    let task_id = Uuid::new_v4();
    let (task_id, _reused, item_ids) = db
        .create_task_submission(
            task_id,
            user_id,
            None,
            "text_batch",
            Some("test/fast-path"),
            None,
            &[json!({"external_id": "a"})],
            None,
            "fast-path-hash",
        )
        .await
        .expect("create task");
    assert_eq!(item_ids.len(), 1);

    let claimed = db
        .claim_items_fast(10)
        .await
        .expect("fast claim")
        .into_iter()
        .find(|item| item.task_id == task_id)
        .expect("fresh item must be claimable on the fast path");
    assert_eq!(claimed.attempt_count, 1, "fast claim is attempt 1");
    assert_eq!(
        claimed.task_id, task_id,
        "fast claim must return the seeded task"
    );

    let task = db
        .get_task_internal(task_id)
        .await
        .expect("load task")
        .expect("task exists");
    assert_eq!(
        task.status, "running",
        "fast claim must activate the parent task"
    );

    cleanup_task(&db, task_id, user_id).await;
    cleanup_user(&db, user_id).await;
}

#[tokio::test]
async fn claim_items_fast_docling_poll_waiting_remains_claimable_at_and_above_cap() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping docling poll claimable test");
        return;
    };
    let _guard = FAST_PATH_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;

    for attempt in [5, 10] {
        let task_id = Uuid::new_v4();
        let (task_id, _reused, item_ids) = db
            .create_task_submission(
                task_id,
                user_id,
                None,
                "text_batch",
                Some("test/fast-path"),
                None,
                &[json!({"external_id": format!("docling-claim-{attempt}")})],
                None,
                &format!("fast-path-docling-claim-{attempt}-{}", Uuid::new_v4()),
            )
            .await
            .expect("create docling poll task");
        let item_id = item_ids[0];

        // Park the item as a due waiting docling_poll with an active external job.
        // The generic five-attempt cap must not prevent this poll from being claimed.
        sqlx::query(
            "UPDATE context69.task_items \
             SET status = 'waiting', stage = 'docling_poll', waiting_reason = 'external_job', \
                 attempt_count = $2, next_attempt_at = now() - interval '1 minute', waiting_since = now() \
             WHERE id = $1",
        )
        .bind(item_id)
        .bind(attempt)
        .execute(db.pool())
        .await
        .expect("park item as waiting docling_poll");

        sqlx::query(
            "UPDATE context69.tasks \
             SET status = 'waiting', stage = 'docling_poll', waiting_reason = 'external_job', \
                 next_attempt_at = now() - interval '1 minute' \
             WHERE id = $1",
        )
        .bind(task_id)
        .execute(db.pool())
        .await
        .expect("park task as waiting");

        let deadline = chrono::Utc::now() + chrono::Duration::hours(1);
        let next_poll = chrono::Utc::now() - chrono::Duration::seconds(30);
        let job_id =
            insert_docling_external_job(&db, item_id, "pending", next_poll, deadline).await;

        let claimed = db
            .claim_items_fast(10)
            .await
            .expect("fast claim")
            .into_iter()
            .find(|item| item.task_id == task_id)
            .expect("due waiting docling_poll must remain claimable at/above the generic cap");

        assert_eq!(
            claimed.id, item_id,
            "claimed id must match the docling_poll item"
        );
        assert_eq!(
            claimed.attempt_count,
            attempt + 1,
            "claim must increment attempt_count even when the item is past the generic cap"
        );

        // Clean up: remove the external job first so the task can be deleted without FK violation
        // and so the next iteration does not see stale polling rows.
        sqlx::query("DELETE FROM context69.task_external_jobs WHERE id = $1")
            .bind(job_id)
            .execute(db.pool())
            .await
            .expect("clean up external job");
        // The claim moved the item to running, so we need to delete task_attempts as well before deleting the task.
        sqlx::query("DELETE FROM context69.task_attempts WHERE item_id = $1")
            .bind(item_id)
            .execute(db.pool())
            .await
            .expect("clean up attempts");
        cleanup_task(&db, task_id, user_id).await;
    }

    cleanup_user(&db, user_id).await;
}

#[tokio::test]
async fn claim_items_fast_docling_poll_respects_due_and_terminal_paths() {
    let Some(url) = test_database_url() else {
        eprintln!(
            "CONTEXT69_TEST_DATABASE_URL is not set; skipping docling poll due/terminal test"
        );
        return;
    };
    let _guard = FAST_PATH_LOCK.lock().await;
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let user_id = seed_test_user(&db).await;

    // A waiting docling_poll that is not yet due must not be claimed even
    // though the generic cap is bypassed; the next_attempt_at gate still applies.
    let not_due_task = Uuid::new_v4();
    let (not_due_task, _reused, not_due_items) = db
        .create_task_submission(
            not_due_task,
            user_id,
            None,
            "text_batch",
            Some("test/fast-path"),
            None,
            &[json!({"external_id": "not-due"})],
            None,
            &format!("fast-path-not-due-{}", Uuid::new_v4()),
        )
        .await
        .expect("create not-due task");
    let not_due_item = not_due_items[0];
    sqlx::query(
        "UPDATE context69.task_items \
         SET status = 'waiting', stage = 'docling_poll', waiting_reason = 'external_job', \
             attempt_count = 5, next_attempt_at = now() + interval '5 minutes', waiting_since = now() \
         WHERE id = $1",
    )
    .bind(not_due_item)
    .execute(db.pool())
    .await
    .expect("park not-due item");
    sqlx::query(
        "UPDATE context69.tasks SET status = 'waiting', stage = 'docling_poll', waiting_reason = 'external_job', \
             next_attempt_at = now() + interval '5 minutes' WHERE id = $1",
    )
    .bind(not_due_task)
    .execute(db.pool())
    .await
    .expect("park not-due task");
    let deadline = chrono::Utc::now() + chrono::Duration::hours(1);
    let next_poll = chrono::Utc::now() - chrono::Duration::seconds(30);
    let not_due_job =
        insert_docling_external_job(&db, not_due_item, "running", next_poll, deadline).await;

    let claimed_not_due = db.claim_items_fast(10).await.expect("fast claim not-due");
    assert!(
        claimed_not_due
            .iter()
            .all(|item| item.task_id != not_due_task),
        "a waiting docling_poll whose next_attempt_at is in the future must not be claimable"
    );

    sqlx::query("DELETE FROM context69.task_external_jobs WHERE id = $1")
        .bind(not_due_job)
        .execute(db.pool())
        .await
        .expect("clean up not-due job");
    cleanup_task(&db, not_due_task, user_id).await;

    // A due waiting docling_poll with a terminal external job (failure) must
    // still be claimable so the poll code can observe the terminal state and
    // resubmit through the existing path rather than being exhausted.
    let terminal_task = Uuid::new_v4();
    let (terminal_task, _reused, terminal_items) = db
        .create_task_submission(
            terminal_task,
            user_id,
            None,
            "text_batch",
            Some("test/fast-path"),
            None,
            &[json!({"external_id": "terminal"})],
            None,
            &format!("fast-path-terminal-{}", Uuid::new_v4()),
        )
        .await
        .expect("create terminal task");
    let terminal_item = terminal_items[0];
    sqlx::query(
        "UPDATE context69.task_items \
         SET status = 'waiting', stage = 'docling_poll', waiting_reason = 'external_job', \
             attempt_count = 6, next_attempt_at = now() - interval '1 minute', waiting_since = now() \
         WHERE id = $1",
    )
    .bind(terminal_item)
    .execute(db.pool())
    .await
    .expect("park terminal item");
    sqlx::query(
        "UPDATE context69.tasks SET status = 'waiting', stage = 'docling_poll', waiting_reason = 'external_job', \
             next_attempt_at = now() - interval '1 minute' WHERE id = $1",
    )
    .bind(terminal_task)
    .execute(db.pool())
    .await
    .expect("park terminal task");
    let terminal_job = insert_docling_external_job(
        &db,
        terminal_item,
        "failure",
        chrono::Utc::now() - chrono::Duration::seconds(30),
        deadline,
    )
    .await;

    let claimed_terminal = db
        .claim_items_fast(10)
        .await
        .expect("fast claim terminal")
        .into_iter()
        .find(|item| item.task_id == terminal_task)
        .expect("due waiting docling_poll with a terminal external job must remain claimable");

    assert_eq!(claimed_terminal.id, terminal_item);

    sqlx::query("DELETE FROM context69.task_external_jobs WHERE id = $1")
        .bind(terminal_job)
        .execute(db.pool())
        .await
        .expect("clean up terminal job");
    sqlx::query("DELETE FROM context69.task_attempts WHERE item_id = $1")
        .bind(terminal_item)
        .execute(db.pool())
        .await
        .expect("clean up attempts");
    cleanup_task(&db, terminal_task, user_id).await;

    // A due waiting docling_poll with no external job row at all (missing
    // remote job, e.g. after a 404) must also remain claimable so the worker
    // can detect the missing job and resubmit.
    let missing_task = Uuid::new_v4();
    let (missing_task, _reused, missing_items) = db
        .create_task_submission(
            missing_task,
            user_id,
            None,
            "text_batch",
            Some("test/fast-path"),
            None,
            &[json!({"external_id": "missing"})],
            None,
            &format!("fast-path-missing-{}", Uuid::new_v4()),
        )
        .await
        .expect("create missing task");
    let missing_item = missing_items[0];
    sqlx::query(
        "UPDATE context69.task_items \
         SET status = 'waiting', stage = 'docling_poll', waiting_reason = 'external_job', \
             attempt_count = 7, next_attempt_at = now() - interval '1 minute', waiting_since = now() \
         WHERE id = $1",
    )
    .bind(missing_item)
    .execute(db.pool())
    .await
    .expect("park missing item");
    sqlx::query(
        "UPDATE context69.tasks SET status = 'waiting', stage = 'docling_poll', waiting_reason = 'external_job', \
             next_attempt_at = now() - interval '1 minute' WHERE id = $1",
    )
    .bind(missing_task)
    .execute(db.pool())
    .await
    .expect("park missing task");

    let claimed_missing = db
        .claim_items_fast(10)
        .await
        .expect("fast claim missing")
        .into_iter()
        .find(|item| item.task_id == missing_task)
        .expect("due waiting docling_poll with a missing external job must remain claimable");

    assert_eq!(claimed_missing.id, missing_item);

    sqlx::query("DELETE FROM context69.task_attempts WHERE item_id = $1")
        .bind(missing_item)
        .execute(db.pool())
        .await
        .expect("clean up attempts");
    cleanup_task(&db, missing_task, user_id).await;
    cleanup_user(&db, user_id).await;
}
