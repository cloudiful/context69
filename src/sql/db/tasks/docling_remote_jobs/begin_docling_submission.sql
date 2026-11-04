-- Persist one Docling dispatch intent before any HTTP send (issue 1172 P5A).
--
-- The row is inserted as `submitting` with no remote id: the frozen identity,
-- the captured execution (library task + generation) and the endpoint snapshot
-- are durable before the worker is allowed to prepare or send the request.
-- The item guard is the captured execution of the worker, so a stale worker
-- never creates an intent for an item it no longer owns.
--
-- The unresolved unique index bounds one open submission per item, so a
-- concurrent loser inserts nothing and must reread the winner instead of
-- sending a second request.
--
-- `$1` item, `$2` domain task, `$3` fencing token, `$4` library task,
-- `$5` execution generation, `$6` endpoint base URL, `$7` frozen identity and
-- `$8` deadline.
WITH owned AS (
    SELECT item.id
    FROM context69.task_items AS item
    WHERE item.id = $1
      AND item.task_id = $2
      AND item.status = 'running'
      AND item.lease_token = $3
      AND item.lease_until > clock_timestamp()
      AND item.execution_task_id = $4
      AND item.execution_generation = $5
)
INSERT INTO context69.task_docling_remote_jobs (
    task_id,
    item_id,
    provider,
    remote_task_id,
    status,
    dispatch_attempted,
    submission_execution_task_id,
    submission_generation,
    submission_identity,
    endpoint_base_url,
    deadline_at
)
SELECT
    $2,
    $1,
    'docling',
    NULL,
    'submitting',
    false,
    $4,
    $5,
    $7,
    $6,
    $8
FROM owned
ON CONFLICT DO NOTHING
RETURNING
    id,
    task_id,
    item_id,
    provider,
    remote_task_id,
    status,
    remote_status,
    attempt_count,
    poll_step,
    next_poll_at,
    clock_timestamp() AS "observed_at!",
    last_polled_at,
    deadline_at,
    endpoint_base_url,
    lease_token,
    lease_until,
    last_error,
    created_at,
    updated_at,
    finished_at,
    dispatch_attempted,
    submission_execution_task_id,
    submission_generation,
    submission_identity
