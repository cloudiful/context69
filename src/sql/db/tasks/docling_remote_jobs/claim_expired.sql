-- Expiry discovery for the Docling poll sweep (issue 639 P2-1).
--
-- Returns up to $1 active remote jobs whose deadline has passed, locking
-- them with FOR UPDATE SKIP LOCKED so concurrent sweep instances never
-- finalize the same expiry twice. This statement deliberately does NOT mark
-- anything terminal: the caller finalizes each returned row atomically via
-- `finish_without_lease.sql` + `fail_waiting_item.sql` + file projection in
-- a single transaction. A crash between this read and that commit leaves the
-- row active, so the next sweep retries it; the dispatcher exclusion for
-- waiting/docling items (`claim_items.sql`) keeps the parked item away from
-- normal workers in the meantime. There is no window where the remote row
-- reads terminal while its item is still parked.
SELECT
    id,
    task_id,
    item_id,
    provider,
    remote_task_id,
    status,
    remote_status,
    attempt_count,
    next_poll_at,
    last_polled_at,
    deadline_at,
    lease_token,
    lease_until,
    last_error,
    created_at,
    updated_at,
    finished_at
FROM context69.task_docling_remote_jobs
WHERE status IN ('pending', 'running')
  AND deadline_at IS NOT NULL
  AND deadline_at <= now()
ORDER BY deadline_at, id
LIMIT $1
FOR UPDATE SKIP LOCKED
