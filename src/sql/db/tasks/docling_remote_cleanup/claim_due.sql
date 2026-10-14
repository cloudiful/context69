-- Claims up to $1 due cleanup rows with FOR UPDATE SKIP LOCKED.
--
-- Due means status in ('pending','cancelling','awaiting_terminal'),
-- next_attempt_at <= now() and no live claim; expired deadlines are settled failed.
-- Sets a 60s claim lease with the caller's token ($2) and bumps the attempt
-- count so backoff and caps are observable. Two concurrent claimers never
-- receive the same row. No network happens here.
WITH due AS (
    SELECT id
    FROM context69.task_docling_remote_cleanup
    WHERE status IN ('pending', 'cancelling', 'awaiting_terminal')
      AND next_attempt_at <= now()
      AND (claim_until IS NULL OR claim_until <= now())
    ORDER BY next_attempt_at, created_at, id
    LIMIT $1
    FOR UPDATE SKIP LOCKED
)
UPDATE context69.task_docling_remote_cleanup AS cleanup
SET claim_token = $2,
    claim_until = now() + interval '60 seconds',
    attempt_count = attempt_count + 1,
    updated_at = now()
FROM due
WHERE cleanup.id = due.id
RETURNING
    cleanup.id,
    cleanup.remote_job_id,
    cleanup.task_id,
    cleanup.item_id,
    cleanup.remote_task_id,
    cleanup.endpoint_base_url,
    cleanup.reason,
    cleanup.status,
    cleanup.outcome,
    cleanup.attempt_count,
    cleanup.next_attempt_at,
    cleanup.created_at,
    cleanup.updated_at,
    cleanup.finished_at,
    cleanup.cleanup_deadline_at,
    cleanup.claim_token,
    cleanup.claim_until,
    cleanup.last_error;
