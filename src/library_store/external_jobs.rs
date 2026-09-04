use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

use super::LibraryStore;

#[derive(Debug, Clone, FromRow)]
pub(crate) struct StoredExternalJob {
    pub id: Uuid,
    pub remote_task_id: String,
    pub status: String,
    pub remote_status: Option<String>,
    pub submitted_at: DateTime<Utc>,
    pub next_poll_at: DateTime<Utc>,
    pub deadline_at: Option<DateTime<Utc>>,
    pub error_message: Option<String>,
    pub submission_count: i32,
}

impl StoredExternalJob {
    pub(crate) fn is_active(&self) -> bool {
        matches!(self.status.as_str(), "pending" | "running")
    }

    pub(crate) fn is_submitting(&self) -> bool {
        self.status == "submitting"
    }

    pub(crate) fn is_orphaned(&self) -> bool {
        self.status == "orphaned"
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SupersededExternalJob {
    pub old_external_job_id: Option<Uuid>,
    pub old_remote_task_id: Option<String>,
    pub old_remote_status: Option<String>,
    pub prior_submission_count: i32,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ExternalJobSubmission {
    pub id: Uuid,
    pub submission_count: i32,
}

pub(crate) struct RecoveryAudit<'a> {
    pub task_id: Uuid,
    pub item_id: Uuid,
    pub actor_user_id: i64,
    pub actor_login_name: &'a str,
    pub reason: &'a str,
    pub old_external_job_id: Option<Uuid>,
    pub old_remote_task_id: Option<&'a str>,
    pub old_remote_status: Option<&'a str>,
    pub old_submission_count: i32,
    pub new_external_job_id: Uuid,
    pub new_remote_task_id: &'a str,
    pub new_submission_count: i32,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct DoclingAdmissionDenied {
    pub inflight: i64,
    pub limit: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DoclingPollDenyReason {
    RateLimited,
    AlreadyReserved,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct DoclingPollDenied {
    pub recent: i64,
    pub limit: usize,
    pub reason: DoclingPollDenyReason,
}

/// One `submitting` row isolated as `orphaned` by the admin quarantine API.
#[derive(Debug, Clone, FromRow)]
pub(crate) struct StoredQuarantinedExternalJob {
    pub external_job_id: Uuid,
    pub item_id: Uuid,
    pub task_id: Uuid,
    pub old_remote_task_id: Option<String>,
    pub quarantined_at: Option<DateTime<Utc>>,
}

/// Eligibility breakdown for uncertain `submitting` rows. The first five
/// buckets partition every `submitting` row exactly once; `orphaned` counts
/// rows already isolated by a previous quarantine call.
#[derive(Debug, Clone, FromRow)]
pub(crate) struct StoredSubmittingQuarantineStats {
    pub uncertain_submitting_count: i64,
    pub quarantinable_count: i64,
    pub skipped_non_terminal_count: i64,
    pub skipped_fresh_count: i64,
    pub skipped_real_remote_count: i64,
    pub orphaned_count: i64,
}

/// Local placeholder prefix for remote ids that never left this service.
/// `try_begin_external_job_submission` inserts `submitting-<uuid>` before the
/// POST; only ids with this prefix prove no real remote job can exist.
pub(crate) const SUBMITTING_PLACEHOLDER_PATTERN: &str = "submitting-%";

impl LibraryStore {
    #[cfg(test)]
    pub(crate) async fn count_docling_inflight(&self, provider: &str) -> anyhow::Result<i64> {
        Ok(sqlx::query_file_scalar!(
            "src/sql/library_store/external_jobs/count_inflight.sql",
            provider,
        )
        .fetch_one(self.db.pool())
        .await?)
    }

    /// Atomically claim a Docling remote slot and insert the `submitting` row.
    ///
    /// Holds the singleton `docling_settings` row `FOR UPDATE` so concurrent
    /// submitters across processes serialize their check-and-insert. Counts
    /// remote non-terminal jobs (`pending`/`running`) plus fresh `submitting`
    /// reservations; stale `submitting` leftovers are ignored so they cannot
    /// wedge admission (phase 4 owns their cleanup). Returns `Ok(Err(denied))`
    /// without inserting when the persistent `max_inflight` ceiling is reached;
    /// the caller must wait and must not POST to Docling.
    pub(crate) async fn try_begin_external_job_submission(
        &self,
        item_id: Uuid,
        provider: &str,
        remote_task_id: &str,
        next_poll_at: DateTime<Utc>,
        deadline_at: DateTime<Utc>,
    ) -> anyhow::Result<Result<ExternalJobSubmission, DoclingAdmissionDenied>> {
        let mut tx = self.db.pool().begin().await?;
        // Serialize admissions even when the settings row is missing (no row
        // to lock); the settings-row lock below then handles the configured
        // case with a second, row-level serialization point.
        sqlx::query_file!("src/sql/library_store/external_jobs/lock_docling_admission.sql")
            .execute(&mut *tx)
            .await?;
        let limit_row: Option<i64> = sqlx::query_file_scalar!(
            "src/sql/db/docling_settings/get_max_inflight_for_update.sql",
        )
        .fetch_optional(&mut *tx)
        .await?;
        let limit = limit_row
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(context69_contracts::settings::DOCLING_MAX_INFLIGHT_DEFAULT)
            .clamp(
                context69_contracts::settings::DOCLING_MAX_INFLIGHT_MIN,
                context69_contracts::settings::DOCLING_MAX_INFLIGHT_MAX,
            );
        let inflight: i64 = sqlx::query_file_scalar!(
            "src/sql/library_store/external_jobs/count_inflight.sql",
            provider,
        )
        .fetch_one(&mut *tx)
        .await?;
        if inflight >= limit as i64 {
            tx.rollback().await?;
            return Ok(Err(DoclingAdmissionDenied { inflight, limit }));
        }
        let row = sqlx::query_file!(
            "src/sql/library_store/external_jobs/begin_submission.sql",
            item_id,
            provider,
            remote_task_id,
            next_poll_at,
            deadline_at,
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(Ok(ExternalJobSubmission {
            id: row.id,
            submission_count: row.submission_count,
        }))
    }

    /// Atomically reserve one Docling poll HTTP slot for `job_id`.
    ///
    /// Multi-instance safe counterpart to the in-process
    /// `docling_poll_slots` semaphore: holds a dedicated advisory lock
    /// (727335733, never the submit key 727335732) while it reads the
    /// persisted `max_inflight` ceiling, counts trailing-window poll
    /// reservations, and conditionally reserves this job. Returns
    /// `Ok(Err(denied))` without touching the network when the window is
    /// full (`RateLimited`) or another worker already reserved this poll
    /// (`AlreadyReserved`); the caller must defer without HTTP.
    pub(crate) async fn try_claim_docling_poll(
        &self,
        provider: &str,
        job_id: Uuid,
        reserved_next_poll_at: DateTime<Utc>,
    ) -> anyhow::Result<Result<(), DoclingPollDenied>> {
        let mut tx = self.db.pool().begin().await?;
        sqlx::query_file!("src/sql/library_store/external_jobs/lock_docling_poll.sql")
            .execute(&mut *tx)
            .await?;
        let limit_row: Option<i64> = sqlx::query_file_scalar!(
            "src/sql/db/docling_settings/get_max_inflight_for_update.sql",
        )
        .fetch_optional(&mut *tx)
        .await?;
        let limit = limit_row
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(context69_contracts::settings::DOCLING_MAX_INFLIGHT_DEFAULT)
            .clamp(
                context69_contracts::settings::DOCLING_MAX_INFLIGHT_MIN,
                context69_contracts::settings::DOCLING_MAX_INFLIGHT_MAX,
            );
        let recent: i64 = sqlx::query_file_scalar!(
            "src/sql/library_store/external_jobs/count_recent_polls.sql",
            provider,
        )
        .fetch_one(&mut *tx)
        .await?;
        if recent >= limit as i64 {
            tx.rollback().await?;
            return Ok(Err(DoclingPollDenied {
                recent,
                limit,
                reason: DoclingPollDenyReason::RateLimited,
            }));
        }
        let reserved = sqlx::query_file!(
            "src/sql/library_store/external_jobs/claim_poll_reservation.sql",
            job_id,
            reserved_next_poll_at,
        )
        .fetch_optional(&mut *tx)
        .await?;
        if reserved.is_none() {
            tx.rollback().await?;
            return Ok(Err(DoclingPollDenied {
                recent,
                limit,
                reason: DoclingPollDenyReason::AlreadyReserved,
            }));
        }
        tx.commit().await?;
        Ok(Ok(()))
    }

    pub(crate) async fn complete_external_job_submission(
        &self,
        id: Uuid,
        remote_task_id: &str,
        next_poll_at: DateTime<Utc>,
    ) -> anyhow::Result<ExternalJobSubmission> {
        let row = sqlx::query_file!(
            "src/sql/library_store/external_jobs/complete_submission.sql",
            id,
            remote_task_id,
            next_poll_at,
        )
        .fetch_one(self.db.pool())
        .await?;
        Ok(ExternalJobSubmission {
            id: row.id,
            submission_count: row.submission_count,
        })
    }

    pub(crate) async fn get_external_job(
        &self,
        item_id: Uuid,
        provider: &str,
    ) -> anyhow::Result<Option<StoredExternalJob>> {
        Ok(sqlx::query_file_as!(
            StoredExternalJob,
            "src/sql/library_store/external_jobs/get_external_job.sql",
            item_id,
            provider,
        )
        .fetch_optional(self.db.pool())
        .await?)
    }

    pub(crate) async fn update_external_job(
        &self,
        id: Uuid,
        status: &str,
        remote_status: Option<&str>,
        next_poll_at: DateTime<Utc>,
        error_message: Option<&str>,
    ) -> anyhow::Result<()> {
        sqlx::query_file!(
            "src/sql/library_store/external_jobs/update_external_job.sql",
            id,
            status,
            remote_status,
            next_poll_at,
            error_message,
        )
        .execute(self.db.pool())
        .await?;
        Ok(())
    }

    /// Atomically mark the (item_id, provider) external job as superseded
    /// (cancelled when active, left alone when already terminal) and return
    /// the prior state for the caller to write a recovery audit row.
    pub(crate) async fn supersede_external_job(
        &self,
        item_id: Uuid,
        provider: &str,
        reason: &str,
    ) -> anyhow::Result<SupersededExternalJob> {
        let row = sqlx::query_file!(
            "src/sql/library_store/external_jobs/mark_external_job_superseded.sql",
            item_id,
            provider,
            reason,
        )
        .fetch_one(self.db.pool())
        .await?;
        Ok(SupersededExternalJob {
            old_external_job_id: row.old_external_job_id,
            old_remote_task_id: row.old_remote_task_id,
            old_remote_status: row.old_remote_status,
            prior_submission_count: row.prior_submission_count.unwrap_or(0),
        })
    }

    /// Batch-isolate stale uncertain `submitting` rows as `orphaned`.
    ///
    /// Only placeholder remote ids older than `grace_cutoff` on terminal
    /// parents are touched (see `quarantine_stale_submitting.sql`); live
    /// `pending`/`running` jobs, fresh rows, real remote ids, and
    /// non-terminal parents are left alone. The transition never claims the
    /// remote job was cancelled. One quarantine audit row per job is written
    /// in the same statement.
    pub(crate) async fn quarantine_stale_submitting(
        &self,
        reason: &str,
        quarantined_by: &str,
        actor_user_id: i64,
        grace_cutoff: DateTime<Utc>,
        placeholder_pattern: &str,
        limit: i64,
    ) -> anyhow::Result<Vec<StoredQuarantinedExternalJob>> {
        Ok(sqlx::query_file_as!(
            StoredQuarantinedExternalJob,
            "src/sql/library_store/external_jobs/quarantine_stale_submitting.sql",
            reason,
            quarantined_by,
            grace_cutoff,
            placeholder_pattern,
            limit,
            actor_user_id,
        )
        .fetch_all(self.db.pool())
        .await?)
    }

    /// Count uncertain `submitting` rows by quarantine eligibility so
    /// operators can see what remains and why it was skipped.
    pub(crate) async fn quarantine_submitting_stats(
        &self,
        grace_cutoff: DateTime<Utc>,
        placeholder_pattern: &str,
    ) -> anyhow::Result<StoredSubmittingQuarantineStats> {
        Ok(sqlx::query_file_as!(
            StoredSubmittingQuarantineStats,
            "src/sql/library_store/external_jobs/quarantine_submitting_stats.sql",
            grace_cutoff,
            placeholder_pattern,
        )
        .fetch_one(self.db.pool())
        .await?)
    }

    /// Record a recovery audit row once the new external job has been
    /// submitted and Docling has returned the fresh remote id.
    pub(crate) async fn record_recovery_audit(
        &self,
        audit: &RecoveryAudit<'_>,
    ) -> anyhow::Result<Uuid> {
        let row = sqlx::query_file!(
            "src/sql/library_store/external_jobs/insert_recovery_audit.sql",
            audit.task_id,
            audit.item_id,
            audit.actor_user_id,
            audit.actor_login_name,
            audit.reason,
            audit.old_external_job_id,
            audit.old_remote_task_id,
            audit.old_remote_status,
            audit.old_submission_count,
            audit.new_external_job_id,
            audit.new_remote_task_id,
            audit.new_submission_count,
        )
        .fetch_one(self.db.pool())
        .await?;
        Ok(row.id)
    }
}

#[cfg(test)]
mod admission_tests {
    use chrono::Utc;
    use serde_json::json;
    use sqlx::Row;
    use uuid::Uuid;

    use super::LibraryStore;
    use crate::db::Database;

    static ADMISSION_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    /// Isolated provider so global `docling` jobs from other test files cannot
    /// affect admission counts.
    const TEST_PROVIDER: &str = "docling-admission-test";

    fn test_database_url() -> Option<String> {
        std::env::var("CONTEXT69_TEST_DATABASE_URL").ok()
    }

    async fn seed_user(db: &Database) -> i64 {
        sqlx::query(
            "INSERT INTO context69.users (login_name, display_name, password_hash) \
             VALUES ($1, $2, $3) RETURNING id",
        )
        .bind(format!("admission-test-{}", Uuid::new_v4()))
        .bind("Admission Test")
        .bind("unused")
        .fetch_one(db.pool())
        .await
        .expect("seed user")
        .get("id")
    }

    async fn cleanup_tasks_and_user(db: &Database, task_ids: &[Uuid], user_id: i64) {
        for task_id in task_ids {
            sqlx::query(
                "DELETE FROM context69.task_external_jobs WHERE item_id IN \
                 (SELECT id FROM context69.task_items WHERE task_id = $1)",
            )
            .bind(*task_id)
            .execute(db.pool())
            .await
            .expect("cleanup jobs");
            sqlx::query("DELETE FROM context69.task_items WHERE task_id = $1")
                .bind(*task_id)
                .execute(db.pool())
                .await
                .expect("cleanup items");
            sqlx::query("DELETE FROM context69.tasks WHERE id = $1")
                .bind(*task_id)
                .execute(db.pool())
                .await
                .expect("cleanup task");
        }
        sqlx::query("DELETE FROM context69.task_idempotency_keys WHERE user_id = $1")
            .bind(user_id)
            .execute(db.pool())
            .await
            .expect("cleanup keys");
        sqlx::query("DELETE FROM context69.users WHERE id = $1")
            .bind(user_id)
            .execute(db.pool())
            .await
            .expect("cleanup user");
    }

    async fn stash_max_inflight(db: &Database) -> Option<i64> {
        let current: Option<i64> = sqlx::query_scalar(
            "SELECT max_inflight FROM context69.docling_settings WHERE singleton = TRUE",
        )
        .fetch_optional(db.pool())
        .await
        .expect("read max_inflight");
        if current.is_some() {
            sqlx::query("UPDATE context69.docling_settings SET max_inflight = 1, updated_at = now() WHERE singleton = TRUE")
                .execute(db.pool())
                .await
                .expect("pin max_inflight to 1");
        }
        current
    }

    async fn restore_max_inflight(db: &Database, previous: Option<i64>) {
        if let Some(value) = previous {
            sqlx::query("UPDATE context69.docling_settings SET max_inflight = $1, updated_at = now() WHERE singleton = TRUE")
                .bind(value)
                .execute(db.pool())
                .await
                .expect("restore max_inflight");
        }
    }

    async fn insert_job(
        db: &Database,
        item_id: Uuid,
        provider: &str,
        remote: &str,
        status: &str,
        submitted_offset_secs: i64,
    ) {
        let submitted_at = Utc::now() + chrono::Duration::seconds(submitted_offset_secs);
        sqlx::query(
            "INSERT INTO context69.task_external_jobs \
             (item_id, provider, remote_task_id, status, submitted_at, next_poll_at, deadline_at, submission_count) \
             VALUES ($1, $2, $3, $4, $5, now(), now() + interval '1 hour', 1)",
        )
        .bind(item_id)
        .bind(provider)
        .bind(remote)
        .bind(status)
        .bind(submitted_at)
        .execute(db.pool())
        .await
        .expect("insert job");
    }

    #[tokio::test]
    async fn inflight_count_ignores_terminal_and_stale_submitting() {
        let Some(url) = test_database_url() else {
            eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping admission count test");
            return;
        };
        let _guard = ADMISSION_LOCK.lock().await;
        let db = Database::connect(&url).await.expect("connect");
        let store = LibraryStore::new(db.clone());
        let user_id = seed_user(&db).await;
        let task_id = Uuid::new_v4();
        let (task_id, _reused, item_ids) = db
            .create_task_submission(
                task_id,
                user_id,
                None,
                "text_batch",
                Some("test/admission-count"),
                None,
                &[
                    json!({"external_id": "count-pending"}),
                    json!({"external_id": "count-stale"}),
                    json!({"external_id": "count-terminal"}),
                ],
                None,
                "admission-count-hash",
            )
            .await
            .expect("create task");

        // Sanity: isolated provider starts empty.
        let baseline = store
            .count_docling_inflight(TEST_PROVIDER)
            .await
            .expect("baseline count");
        assert_eq!(baseline, 0, "isolated test provider must start empty");

        insert_job(
            &db,
            item_ids[0],
            TEST_PROVIDER,
            "remote-pending",
            "pending",
            0,
        )
        .await;
        // Stale submitting (2h old) must not count; terminal never counts.
        insert_job(
            &db,
            item_ids[1],
            TEST_PROVIDER,
            "remote-stale",
            "submitting",
            -7200,
        )
        .await;
        insert_job(&db, item_ids[2], TEST_PROVIDER, "remote-done", "success", 0).await;

        let count = store
            .count_docling_inflight(TEST_PROVIDER)
            .await
            .expect("count inflight");
        assert_eq!(
            count, 1,
            "only the pending row must hold a slot (stale submitting + terminal ignored)"
        );

        // Fresh submitting (now) must also hold a slot.
        let fresh_task = Uuid::new_v4();
        let (fresh_task, _reused, fresh_items) = db
            .create_task_submission(
                fresh_task,
                user_id,
                None,
                "text_batch",
                Some("test/admission-fresh"),
                None,
                &[json!({"external_id": "count-fresh"})],
                None,
                "admission-fresh-hash",
            )
            .await
            .expect("create fresh task");
        insert_job(
            &db,
            fresh_items[0],
            TEST_PROVIDER,
            "remote-fresh",
            "submitting",
            0,
        )
        .await;
        let after = store
            .count_docling_inflight(TEST_PROVIDER)
            .await
            .expect("count after fresh");
        assert_eq!(
            after, 2,
            "fresh submitting must hold a slot alongside pending"
        );

        cleanup_tasks_and_user(&db, &[fresh_task, task_id], user_id).await;
    }

    #[tokio::test]
    async fn admission_denies_when_full_and_admits_after_release() {
        let Some(url) = test_database_url() else {
            eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping admission gate test");
            return;
        };
        let _guard = ADMISSION_LOCK.lock().await;
        let db = Database::connect(&url).await.expect("connect");
        let previous = stash_max_inflight(&db).await;
        let store = LibraryStore::new(db.clone());
        let user_id = seed_user(&db).await;
        let task_id = Uuid::new_v4();
        let (task_id, _reused, item_ids) = db
            .create_task_submission(
                task_id,
                user_id,
                None,
                "text_batch",
                Some("test/admission-gate"),
                None,
                &[
                    json!({"external_id": "gate-holder"}),
                    json!({"external_id": "gate-waiter"}),
                ],
                None,
                "admission-gate-hash",
            )
            .await
            .expect("create gate task");

        // Occupy the single slot with a pending remote job on the isolated provider.
        insert_job(
            &db,
            item_ids[0],
            TEST_PROVIDER,
            "remote-holder",
            "pending",
            0,
        )
        .await;

        let next_poll = Utc::now() + chrono::Duration::seconds(30);
        let deadline = Utc::now() + chrono::Duration::hours(1);
        let denied = store
            .try_begin_external_job_submission(
                item_ids[1],
                TEST_PROVIDER,
                "submitting-waiter",
                next_poll,
                deadline,
            )
            .await
            .expect("admission attempt");
        let denied = denied.expect_err("second slot must be denied while full");
        assert_eq!(denied.limit, 1);
        assert_eq!(denied.inflight, 1);

        // Release the slot by moving the holder to terminal.
        sqlx::query(
            "UPDATE context69.task_external_jobs SET status = 'success' \
             WHERE item_id = $1 AND provider = $2",
        )
        .bind(item_ids[0])
        .bind(TEST_PROVIDER)
        .execute(db.pool())
        .await
        .expect("release slot");
        let admitted = store
            .try_begin_external_job_submission(
                item_ids[1],
                TEST_PROVIDER,
                "submitting-waiter-2",
                next_poll,
                deadline,
            )
            .await
            .expect("second attempt")
            .expect("slot must admit after release");

        // The admitted reservation itself must block a further claim.
        let task2 = Uuid::new_v4();
        let (task2, _reused, extra) = db
            .create_task_submission(
                task2,
                user_id,
                None,
                "text_batch",
                Some("test/admission-extra"),
                None,
                &[json!({"external_id": "gate-extra"})],
                None,
                "admission-extra-hash",
            )
            .await
            .expect("create extra task");
        let blocked = store
            .try_begin_external_job_submission(
                extra[0],
                TEST_PROVIDER,
                "submitting-extra",
                next_poll,
                deadline,
            )
            .await
            .expect("extra attempt");
        assert!(
            blocked.is_err(),
            "fresh submitting reservation must hold the slot"
        );
        let _ = admitted;

        restore_max_inflight(&db, previous).await;
        cleanup_tasks_and_user(&db, &[task2, task_id], user_id).await;
    }

    #[tokio::test]
    async fn concurrent_admissions_do_not_exceed_limit() {
        let Some(url) = test_database_url() else {
            eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping concurrent admission test");
            return;
        };
        let _guard = ADMISSION_LOCK.lock().await;
        let db = Database::connect(&url).await.expect("connect");
        let db_second = Database::connect(&url).await.expect("second connect");
        let previous = stash_max_inflight(&db).await;
        let store = LibraryStore::new(db.clone());
        let store_second = LibraryStore::new(db_second.clone());
        let user_id = seed_user(&db).await;
        let task_id = Uuid::new_v4();
        let (task_id, _reused, item_ids) = db
            .create_task_submission(
                task_id,
                user_id,
                None,
                "text_batch",
                Some("test/admission-race"),
                None,
                &[
                    json!({"external_id": "race-a"}),
                    json!({"external_id": "race-b"}),
                ],
                None,
                "admission-race-hash",
            )
            .await
            .expect("create race task");

        let next_poll = Utc::now() + chrono::Duration::seconds(30);
        let deadline = Utc::now() + chrono::Duration::hours(1);
        let (first, second) = tokio::join!(
            store.try_begin_external_job_submission(
                item_ids[0],
                TEST_PROVIDER,
                "submitting-race-a",
                next_poll,
                deadline,
            ),
            store_second.try_begin_external_job_submission(
                item_ids[1],
                TEST_PROVIDER,
                "submitting-race-b",
                next_poll,
                deadline,
            ),
        );
        let first = first.expect("first race");
        let second = second.expect("second race");
        assert_eq!(
            first.is_ok() as u8 + second.is_ok() as u8,
            1,
            "exactly one concurrent admission must win under limit 1"
        );

        restore_max_inflight(&db, previous).await;
        cleanup_tasks_and_user(&db, &[task_id], user_id).await;
    }

    async fn insert_due_poll_job(db: &Database, item_id: Uuid) -> Uuid {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO context69.task_external_jobs \
             (id, item_id, provider, remote_task_id, status, submitted_at, last_polled_at, next_poll_at, deadline_at, submission_count) \
             VALUES ($1, $2, $3, $4, 'running', now() - interval '10 minutes', NULL, now() - interval '1 minute', now() + interval '1 hour', 1)",
        )
        .bind(id)
        .bind(item_id)
        .bind(TEST_PROVIDER)
        .bind(format!("remote-poll-{id}"))
        .execute(db.pool())
        .await
        .expect("insert due poll job");
        id
    }

    #[tokio::test]
    async fn poll_claim_reserves_slot_and_second_claim_for_same_job_defers() {
        let Some(url) = test_database_url() else {
            eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping poll reservation test");
            return;
        };
        let _guard = ADMISSION_LOCK.lock().await;
        let db = Database::connect(&url).await.expect("connect");
        let previous = stash_max_inflight(&db).await;
        let store = LibraryStore::new(db.clone());
        let user_id = seed_user(&db).await;
        let task_id = Uuid::new_v4();
        let (task_id, _reused, item_ids) = db
            .create_task_submission(
                task_id,
                user_id,
                None,
                "text_batch",
                Some("test/poll-reserve"),
                None,
                &[json!({"external_id": "poll-reserve"})],
                None,
                "poll-reserve-hash",
            )
            .await
            .expect("create poll task");
        let job_id = insert_due_poll_job(&db, item_ids[0]).await;

        let reserved = Utc::now() + chrono::Duration::seconds(35);
        store
            .try_claim_docling_poll(TEST_PROVIDER, job_id, reserved)
            .await
            .expect("first poll claim")
            .expect("first due poll must be claimable when the window is empty");

        let retry_reserved = Utc::now() + chrono::Duration::seconds(35);
        let denied = store
            .try_claim_docling_poll(TEST_PROVIDER, job_id, retry_reserved)
            .await
            .expect("second poll claim");
        // Under limit 1 the first reservation fills the trailing window, so
        // the immediate second claim defers regardless of reason; it must
        // never trigger a second HTTP for the same poll window.
        assert!(
            denied.is_err(),
            "second claim for the same poll window must defer without HTTP"
        );

        restore_max_inflight(&db, previous).await;
        cleanup_tasks_and_user(&db, &[task_id], user_id).await;
    }

    #[tokio::test]
    async fn poll_claim_rate_limits_across_jobs_under_single_slot() {
        let Some(url) = test_database_url() else {
            eprintln!("CONTEXT69_TEST_DATABASE_URL is not set; skipping poll rate-limit test");
            return;
        };
        let _guard = ADMISSION_LOCK.lock().await;
        let db = Database::connect(&url).await.expect("connect");
        let previous = stash_max_inflight(&db).await;
        let store = LibraryStore::new(db.clone());
        let user_id = seed_user(&db).await;
        let task_id = Uuid::new_v4();
        let (task_id, _reused, item_ids) = db
            .create_task_submission(
                task_id,
                user_id,
                None,
                "text_batch",
                Some("test/poll-rate"),
                None,
                &[
                    json!({"external_id": "poll-rate-a"}),
                    json!({"external_id": "poll-rate-b"}),
                ],
                None,
                "poll-rate-hash",
            )
            .await
            .expect("create poll rate task");
        let first_job = insert_due_poll_job(&db, item_ids[0]).await;
        let second_job = insert_due_poll_job(&db, item_ids[1]).await;

        let reserved = Utc::now() + chrono::Duration::seconds(35);
        store
            .try_claim_docling_poll(TEST_PROVIDER, first_job, reserved)
            .await
            .expect("first job claim")
            .expect("first job must win the single poll slot");

        let denied = store
            .try_claim_docling_poll(
                TEST_PROVIDER,
                second_job,
                Utc::now() + chrono::Duration::seconds(35),
            )
            .await
            .expect("second job claim");
        let denied = denied.expect_err("second concurrent poll must defer under limit 1");
        assert_eq!(
            denied.reason,
            super::DoclingPollDenyReason::RateLimited,
            "second job must defer as rate-limited, not as already-reserved"
        );
        assert_eq!(denied.limit, 1);

        restore_max_inflight(&db, previous).await;
        cleanup_tasks_and_user(&db, &[task_id], user_id).await;
    }
}
