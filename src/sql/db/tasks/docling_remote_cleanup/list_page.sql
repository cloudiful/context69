-- One bounded page of the persisted local cleanup outbox (safe projection).
--
-- Filters are exact matches on persisted columns. A NULL filter disables that
-- predicate; the remote task id is the caller-normalized exact value. The
-- ordering matches the count statement so ties stay deterministic.
SELECT
    id,
    remote_job_id,
    task_id,
    item_id,
    remote_task_id,
    reason,
    status,
    outcome,
    attempt_count,
    next_attempt_at,
    created_at,
    updated_at,
    finished_at,
    cleanup_deadline_at,
    last_error
FROM context69.task_docling_remote_cleanup
WHERE ($1::text IS NULL OR status = $1)
  AND ($2::text IS NULL OR outcome = $2)
  AND ($3::text IS NULL OR remote_task_id = $3)
ORDER BY created_at DESC, id DESC
LIMIT $4 OFFSET $5;
