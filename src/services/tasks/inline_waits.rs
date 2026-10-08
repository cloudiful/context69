use std::time::Duration;

use anyhow::Result;
use chrono::{DateTime, Utc};
use context69_contracts::TaskKind;
use uuid::Uuid;

use super::TaskService;
use super::item_processors::{ProcessResult, process_item_blocking};
use crate::db::Database;

/// Bound for consecutive inline waits inside one worker run (issue 650 P3).
///
/// A retryable outcome sleeps inline keeping the admitted parent/item lease
/// instead of parking, then re-drives from the entry stage. The budget
/// matches the backoff clamp range so a poison item still parks through the
/// existing `wait_task_item` path (with attempt accounting and recovery)
/// instead of pinning a global parent slot forever.
pub(super) const MAX_INLINE_WAIT_ROUNDS: u32 = 8;

/// Cancel-responsiveness bound for one inline sleep chunk. Every chunk
/// heartbeats the item lease, so a task cancel (or lease reclaim) aborts the
/// wait within this long plus one heartbeat round-trip.
pub(super) const INLINE_WAIT_CHUNK: Duration = Duration::from_secs(10);

/// Classifies one pipeline outcome for the blocking worker: `Some(until)`
/// sleeps inline keeping the admitted parent/item lease, `None` is committed
/// through the existing park/finish path.
pub(super) fn inline_wait_until(
    result: &ProcessResult,
    attempt_count: i32,
    rounds_used: u32,
) -> Option<DateTime<Utc>> {
    if rounds_used >= MAX_INLINE_WAIT_ROUNDS {
        return None;
    }
    match result {
        ProcessResult::Waiting {
            next_attempt_at, ..
        } => Some(*next_attempt_at),
        ProcessResult::Failed {
            retryable: true, ..
        } => Some(backoff_until(attempt_count)),
        _ => None,
    }
}

/// Drives one claimed item, blocking through retryable waits instead of
/// parking: each `Waiting`/retryable-`Failed` outcome sleeps inline (the item
/// lease stays alive under the runtime heartbeat and the parent slot is
/// renewed by the recovery tick) and re-drives from the entry stage, up to
/// [`MAX_INLINE_WAIT_ROUNDS`]. Returns the first outcome that must be
/// committed (success, terminal failure, or an over-budget wait that parks
/// exactly as before).
pub(super) async fn drive_with_inline_waits(
    service: &TaskService,
    kind: TaskKind,
    group: Option<&crate::domain::GroupRecord>,
    task: &crate::db::StoredTask,
    item: &crate::db::ClaimedItem,
) -> Result<ProcessResult> {
    // The working snapshot carries persisted `file_id`/payload progress
    // across inline re-drives, so a retry resumes where the previous drive
    // left off instead of redoing stages (and recreating files). Lease and
    // attempt identity never change mid-claim.
    let mut current = item.clone();
    let mut result = process_item_blocking(service, kind, group, task, &mut current).await?;
    let mut rounds_used = 0u32;
    while let Some(until) = inline_wait_until(&result, item.attempt_count, rounds_used) {
        tracing::debug!(
            target: "task_lifecycle",
            task_id = %item.task_id,
            item_id = %item.id,
            attempt = item.attempt_count,
            round = rounds_used + 1,
            until = %until,
            "task item retryable wait stays inline keeping the admitted lease"
        );
        if !sleep_keep_lease(service.db(), item.id, item.lease_token, until).await? {
            break;
        }
        rounds_used += 1;
        result = process_item_blocking(service, kind, group, task, &mut current).await?;
    }
    Ok(result)
}

/// Sleeps until `until` in [`INLINE_WAIT_CHUNK`] slices while renewing the
/// item lease. Returns `false` when the lease is gone (task cancelled, item
/// reaped, or lease reclaimed): the caller must commit its outcome through
/// the fenced path, which then observes the loss instead of overwriting the
/// new owner's state.
pub(super) async fn sleep_keep_lease(
    db: &Database,
    item_id: Uuid,
    lease_token: Uuid,
    until: DateTime<Utc>,
) -> Result<bool> {
    while Utc::now() < until {
        let chunk = (until - Utc::now())
            .to_std()
            .unwrap_or(Duration::ZERO)
            .min(INLINE_WAIT_CHUNK);
        if chunk.is_zero() {
            break;
        }
        tokio::time::sleep(chunk).await;
        if !db.heartbeat_task_item(item_id, lease_token).await? {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(super) fn backoff_until(attempt_count: i32) -> DateTime<Utc> {
    let attempt = attempt_count.clamp(1, 8) as u32;
    let seconds = 5_i64.saturating_mul(1_i64 << (attempt - 1));
    Utc::now() + chrono::Duration::seconds(seconds.min(300))
}

#[cfg(test)]
mod tests {
    use chrono::{Duration as ChronoDuration, Utc};

    use super::{MAX_INLINE_WAIT_ROUNDS, backoff_until, inline_wait_until};
    use crate::services::tasks::item_processors::ProcessResult;

    fn waiting(seconds_ahead: i64) -> ProcessResult {
        ProcessResult::Waiting {
            reason: "backoff".to_string(),
            dependency_key: None,
            next_attempt_at: Utc::now() + ChronoDuration::seconds(seconds_ahead),
            message: None,
        }
    }

    fn failed(retryable: bool) -> ProcessResult {
        ProcessResult::Failed {
            stage: "docling".to_string(),
            message: "boom".to_string(),
            retryable,
        }
    }

    #[test]
    fn backoff_grows_and_caps_at_five_minutes() {
        let first = backoff_until(1);
        let second = backoff_until(2);
        assert!((second - first).num_seconds() >= 4);
        for attempt in [7, 8, 9, 100] {
            let until = backoff_until(attempt);
            let secs = (until - Utc::now()).num_seconds();
            assert!(
                (290..=300).contains(&secs),
                "attempt {attempt} must cap near 300s, got {secs}"
            );
        }
        let clamped = backoff_until(0);
        assert!((clamped - Utc::now()).num_seconds() >= 4);
    }

    #[test]
    fn retryable_outcomes_wait_inline_until_the_budget_runs_out() {
        assert!(inline_wait_until(&waiting(30), 1, 0).is_some());
        assert!(inline_wait_until(&failed(true), 3, 0).is_some());
        assert!(inline_wait_until(&waiting(30), 1, MAX_INLINE_WAIT_ROUNDS).is_none());
        assert!(inline_wait_until(&failed(true), 3, MAX_INLINE_WAIT_ROUNDS).is_none());
    }

    #[test]
    fn terminal_outcomes_never_wait_inline() {
        assert!(inline_wait_until(&failed(false), 1, 0).is_none());
        assert!(
            inline_wait_until(&ProcessResult::Succeeded(Some("f".to_string())), 1, 0).is_none()
        );
        assert!(
            inline_wait_until(&ProcessResult::Progressed { next: "embedding" }, 1, 0).is_none()
        );
    }

    #[test]
    fn retryable_failure_uses_the_claim_backoff_schedule() {
        let before = Utc::now();
        let until = inline_wait_until(&failed(true), 2, 0).expect("inline wait");
        let secs = (until - before).num_seconds();
        assert!(
            (8..=12).contains(&secs),
            "attempt 2 must wait ~10s inline, got {secs}"
        );
    }

    /// DB-backed lease retention: the inline wait renews the item lease and
    /// returns once the deadline passes, without parking the item.
    #[tokio::test]
    async fn inline_sleep_keeps_a_live_lease_and_releases_nothing() {
        let Some(db) = test_db().await else {
            eprintln!("CONTEXT69_TEST_DATABASE_URL missing; skipping");
            return;
        };
        let (user_id, task_id, item_id, lease) = seed_running_item(&db).await;
        let before: chrono::DateTime<Utc> = lease_until(&db, item_id).await;
        let done =
            super::sleep_keep_lease(&db, item_id, lease, Utc::now() + ChronoDuration::seconds(2))
                .await
                .expect("sleep");
        assert!(done, "a live lease survives the inline wait");
        let after: chrono::DateTime<Utc> = lease_until(&db, item_id).await;
        assert!(after > before, "the inline wait must renew the item lease");
        let status = item_status(&db, item_id).await;
        assert_eq!(status, "running", "the inline wait must not park the item");
        cleanup(&db, task_id, user_id).await;
    }

    /// A past deadline returns immediately without touching the lease.
    #[tokio::test]
    async fn inline_sleep_with_a_past_deadline_returns_immediately() {
        let Some(db) = test_db().await else {
            eprintln!("CONTEXT69_TEST_DATABASE_URL missing; skipping");
            return;
        };
        let (user_id, task_id, item_id, lease) = seed_running_item(&db).await;
        let done =
            super::sleep_keep_lease(&db, item_id, lease, Utc::now() - ChronoDuration::seconds(1))
                .await
                .expect("sleep");
        assert!(done);
        cleanup(&db, task_id, user_id).await;
    }

    /// Cancel (lease cleared, item terminal) aborts the inline wait at the
    /// next chunk instead of sleeping through the whole backoff.
    #[tokio::test]
    async fn inline_sleep_aborts_when_the_lease_is_gone() {
        let Some(db) = test_db().await else {
            eprintln!("CONTEXT69_TEST_DATABASE_URL missing; skipping");
            return;
        };
        let (user_id, task_id, item_id, lease) = seed_running_item(&db).await;
        sqlx::query(
            "UPDATE context69.task_items SET status = 'cancelled', lease_token = NULL, \
             lease_until = NULL, finished_at = now(), updated_at = now() WHERE id = $1",
        )
        .bind(item_id)
        .execute(db.pool())
        .await
        .expect("cancel item");
        let started = std::time::Instant::now();
        let done = super::sleep_keep_lease(
            &db,
            item_id,
            lease,
            Utc::now() + ChronoDuration::seconds(25),
        )
        .await
        .expect("sleep");
        assert!(!done, "a lost lease must abort the inline wait");
        assert!(
            started.elapsed() < std::time::Duration::from_secs(25),
            "the wait must abort at the first chunk, not sleep through"
        );
        cleanup(&db, task_id, user_id).await;
    }

    async fn test_db() -> Option<crate::db::Database> {
        let url = std::env::var("CONTEXT69_TEST_DATABASE_URL").ok()?;
        Some(
            crate::db::Database::connect(&url)
                .await
                .expect("connect test database"),
        )
    }

    async fn seed_running_item(
        db: &crate::db::Database,
    ) -> (i64, uuid::Uuid, uuid::Uuid, uuid::Uuid) {
        use serde_json::json;
        use sqlx::Row;

        use crate::db::CreateTaskSubmissionRequest;

        let user_id: i64 = sqlx::query(
            "INSERT INTO context69.users (login_name, display_name, password_hash) \
             VALUES ($1, $2, $3) RETURNING id",
        )
        .bind(format!("inline-waits-{}", uuid::Uuid::new_v4()))
        .bind("Inline Waits Test")
        .bind("unused")
        .fetch_one(db.pool())
        .await
        .expect("seed user")
        .get("id");
        let task_id = uuid::Uuid::new_v4();
        let (_, _, items) = db
            .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
                task_id,
                user_id,
                group_id: None,
                kind: "text_batch",
                group_path: Some("test/inline-waits"),
                source_key: None,
                payloads: &[json!({"external_id": "a"})],
                input_storage_object_ids: None,
                idempotency_key: None,
                request_hash: &format!("inline-waits-{}", uuid::Uuid::new_v4()),
            })
            .await
            .expect("create task");
        let item_id = items[0];
        let lease = uuid::Uuid::new_v4();
        sqlx::query(
            "UPDATE context69.task_items SET status = 'running', lease_token = $2, \
             lease_until = now() + interval '5 minutes', attempt_count = 1 WHERE id = $1",
        )
        .bind(item_id)
        .bind(lease)
        .execute(db.pool())
        .await
        .expect("claim-like setup");
        (user_id, task_id, item_id, lease)
    }

    async fn lease_until(db: &crate::db::Database, item_id: uuid::Uuid) -> chrono::DateTime<Utc> {
        use sqlx::Row;

        sqlx::query("SELECT lease_until FROM context69.task_items WHERE id = $1")
            .bind(item_id)
            .fetch_one(db.pool())
            .await
            .expect("read lease")
            .get("lease_until")
    }

    async fn item_status(db: &crate::db::Database, item_id: uuid::Uuid) -> String {
        use sqlx::Row;

        sqlx::query("SELECT status FROM context69.task_items WHERE id = $1")
            .bind(item_id)
            .fetch_one(db.pool())
            .await
            .expect("read status")
            .get("status")
    }

    async fn cleanup(db: &crate::db::Database, task_id: uuid::Uuid, user_id: i64) {
        sqlx::query("DELETE FROM context69.task_items WHERE task_id = $1")
            .bind(task_id)
            .execute(db.pool())
            .await
            .expect("cleanup items");
        sqlx::query("DELETE FROM context69.tasks WHERE id = $1")
            .bind(task_id)
            .execute(db.pool())
            .await
            .expect("cleanup task");
        sqlx::query("DELETE FROM context69.task_idempotency_keys WHERE user_id = $1")
            .bind(user_id)
            .execute(db.pool())
            .await
            .expect("cleanup idempotency");
        sqlx::query("DELETE FROM context69.users WHERE id = $1")
            .bind(user_id)
            .execute(db.pool())
            .await
            .expect("cleanup user");
    }
}
