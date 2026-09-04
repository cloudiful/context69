-- Batch-quarantine stale uncertain `submitting` Docling rows (issue #118 phase 4).
--
-- Only rows that satisfy ALL of the following are moved to `orphaned`:
--   * provider = 'docling' and status = 'submitting';
--   * `remote_task_id` matches the local placeholder pattern ($4,
--     e.g. 'submitting-%'): proof that no real remote id was ever recorded,
--     so no live remote job can be orphaned silently;
--   * `submitted_at` is older than the caller-supplied grace cutoff ($3):
--     the POST outcome window has long passed;
--   * the parent item AND the parent task are both terminal
--     (succeeded/failed/cancelled): nothing will ever poll or resubmit them.
--
-- Everything else is left untouched: `pending`/`running` (live remote jobs),
-- fresh `submitting` rows, rows carrying a real (non-placeholder) remote id,
-- and rows on non-terminal parents. `orphaned` is a non-active status, so a
-- quarantined row stops blocking terminal-task cleanup/purge and stops
-- counting toward Docling admission. The transition never claims the remote
-- job was cancelled: the original `remote_status`/`error_message` are
-- preserved (the reason is appended, never overwritten) and the quarantine
-- actor/timestamp are recorded on the row plus one audit row per job.
--
-- $1 reason, $2 quarantined_by (actor login), $3 grace cutoff (timestamptz),
-- $4 placeholder remote-id pattern (LIKE), $5 row limit, $6 actor user id.
WITH candidates AS (
    SELECT job.id,
           job.item_id,
           job.remote_task_id,
           job.remote_status,
           job.error_message,
           item.task_id
    FROM context69.task_external_jobs job
    JOIN context69.task_items item ON item.id = job.item_id
    JOIN context69.tasks task ON task.id = item.task_id
    WHERE job.provider = 'docling'
      AND job.status = 'submitting'
      AND job.remote_task_id LIKE $4
      AND job.submitted_at < $3
      AND item.status IN ('succeeded', 'failed', 'cancelled')
      AND task.status IN ('succeeded', 'failed', 'cancelled')
    ORDER BY job.submitted_at
    LIMIT $5
    FOR UPDATE OF job SKIP LOCKED
), quarantined AS (
    UPDATE context69.task_external_jobs job
    SET status = 'orphaned',
        remote_status = COALESCE(job.remote_status, job.status),
        error_message = CASE
            WHEN job.error_message IS NULL OR job.error_message = '' THEN $1
            ELSE job.error_message || ' | quarantined: ' || $1
        END,
        quarantine_reason = $1,
        quarantined_at = now(),
        quarantined_by = $2,
        last_polled_at = now(),
        updated_at = now()
    FROM candidates
    WHERE job.id = candidates.id
    RETURNING job.id,
              job.item_id,
              candidates.task_id,
              job.remote_task_id,
              job.quarantined_at
), audits AS (
    INSERT INTO context69.task_external_job_quarantine_audit (
        task_id,
        item_id,
        external_job_id,
        provider,
        old_remote_task_id,
        old_remote_status,
        old_error_message,
        actor_user_id,
        actor_login_name,
        reason
    )
    SELECT quarantined.task_id,
           quarantined.item_id,
           quarantined.id,
           'docling',
           candidates.remote_task_id,
           candidates.remote_status,
           candidates.error_message,
           $6,
           $2,
           $1
    FROM quarantined
    JOIN candidates ON candidates.id = quarantined.id
    RETURNING 1
)
SELECT quarantined.id AS external_job_id,
       quarantined.item_id,
       quarantined.task_id,
       quarantined.remote_task_id AS old_remote_task_id,
       quarantined.quarantined_at
FROM quarantined
ORDER BY quarantined.quarantined_at;
