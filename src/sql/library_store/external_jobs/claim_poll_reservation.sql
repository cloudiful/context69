-- Atomically reserve this job's poll HTTP slot (issue #118 poll-limits).
--
-- Sets `last_polled_at=now()` so concurrent claimants in the trailing
-- window observe this reservation via `count_recent_polls.sql`, and pushes
-- `next_poll_at` to the caller-computed reservation so a raced second
-- claimant for the same job sees `next_poll_at > now()` and defers. Returns
-- zero rows when another worker already reserved this poll window.
UPDATE context69.task_external_jobs
SET last_polled_at = now(),
    next_poll_at = $2,
    updated_at = now()
WHERE id = $1
  AND next_poll_at <= now()
RETURNING id
