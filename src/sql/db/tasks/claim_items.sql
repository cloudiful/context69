-- Fast claim path used by the dispatcher on notification-driven wakes.
--
-- One FIFO shape only (issue 529 Task 4): the eligible selection is the
-- oldest due item (`ORDER BY ti.created_at, ti.id`) with the generic
-- attempt/lease/due gates and no per-kind exemptions. The stage machine is
-- collapsed, the worker runs the whole item inline in one claim, and
-- `task_items.stage` stays at `processing` until the item finishes (a
-- succeeded item lands on the terminal `finalize` stage), so no stage can be
-- prioritised, exempted, or guarded any more. The single blocking worker
-- (scheduler.max_concurrency = 1) also makes the former `file_id`
-- sibling/ordering guards dead: no second claim can run while one item is in
-- flight.
--
-- This intentionally contains only the eligible selection (with FOR
-- UPDATE SKIP LOCKED), parent activation for claimed task_ids, item
-- lease/attempt fields, task_attempts insert, and the returned
-- ClaimedItem. The exhausted-item/file/task propagation and the
-- recovery-attempt interruption live in maintain_claim_state.sql and
-- are not run on every notification. claim_items (the compatibility
-- method) still runs both files in one transaction so existing callers
-- and lease/retry tests observe the same exhaustive behavior.
--
-- The expired-attempt interruption here is scoped to the items being
-- claimed so the fast path still recycles a crashed worker's attempt
-- even when no recovery maintenance has run recently. maintain_claim_state
-- handles the wider expired-attempt set on the recovery tick.
--
-- Waiting tasks are still claimed once their `next_attempt_at` is due: the
-- task-level wait mirrors the earliest waiting item, so backoff and the
-- vector-rebuild resource wait both resume through this predicate.
--
-- Durable Docling remote jobs own their items while active (issue 639): an
-- item with a ('pending','running') row in `task_docling_remote_jobs` is
-- polled by the docling-poll-sweep scheduler job, never by this dispatcher.
-- The NOT EXISTS guard prevents resubmit/double-claim across restarts and
-- replicas; the sweep requeues the item once the remote job is terminal.
--
-- The waiting/docling exclusion below is the crash-window guard (issue 639
-- P2-1): the sweep finalizes expired remote jobs atomically per job (remote
-- finish + item fail + file projection in one transaction), but between the
-- expiry claim and that single commit the remote row may already read
-- terminal while the item is still waiting on Docling. Excluding every
-- waiting/docling item here — regardless of remote-row state — keeps the
-- dispatcher from resubmitting an item the sweep still owns. Only the sweep
-- moves such items (requeue on success, fail on terminal/error).
WITH eligible AS (
    SELECT ti.id, ti.task_id
    FROM context69.task_items ti
    JOIN context69.tasks task ON task.id = ti.task_id
    WHERE (
            task.status IN ('queued', 'running')
            OR (
                task.status = 'waiting'
                AND (task.next_attempt_at IS NULL OR task.next_attempt_at <= now())
            )
        )
      AND (
          (
              ti.status IN ('queued', 'waiting')
              AND ti.attempt_count < 5
              AND (ti.next_attempt_at IS NULL OR ti.next_attempt_at <= now())
          )
          OR (
              ti.status = 'running'
              AND (ti.lease_until IS NULL OR ti.lease_until < now())
          )
      )
      AND NOT EXISTS (
          SELECT 1
          FROM context69.task_docling_remote_jobs remote
          WHERE remote.item_id = ti.id
            AND remote.status IN ('pending', 'running')
      )
      -- NULL-safe: only waiting/docling rows are excluded. A plain
      -- `waiting_reason = 'docling'` comparison yields NULL (not true) for
      -- ordinary waiting rows with a NULL reason, and `NOT NULL` would
      -- wrongly filter them out of the dispatcher.
      AND NOT (
          ti.status = 'waiting'
          AND COALESCE(ti.waiting_reason, '') = 'docling'
      )
    ORDER BY ti.created_at, ti.id
    LIMIT $1
    FOR UPDATE OF ti SKIP LOCKED
), activated AS (
    UPDATE context69.tasks AS task
    SET status = 'running',
        started_at = coalesce(task.started_at, now()),
        updated_at = now()
    WHERE task.id IN (SELECT task_id FROM eligible)
      AND (
          task.status = 'queued'
          OR (
              task.status = 'waiting'
              AND (task.next_attempt_at IS NULL OR task.next_attempt_at <= now())
          )
      )
), expired AS (
    UPDATE context69.task_attempts AS attempt
    SET status = 'interrupted',
        failure_stage = 'lease',
        error_message = 'worker lease expired before completion',
        finished_at = now()
    FROM eligible
    JOIN context69.task_items item ON item.id = eligible.id
    WHERE attempt.item_id = eligible.id
      AND item.status = 'running'
      AND (item.lease_until IS NULL OR item.lease_until < now())
      AND attempt.finished_at IS NULL
), claimed AS (
    UPDATE context69.task_items AS item
    SET status = 'running',
        attempt_count = item.attempt_count + 1,
        lease_token = gen_random_uuid(),
        lease_until = now() + interval '5 minutes',
        started_at = coalesce(item.started_at, now()),
        finished_at = NULL,
        waiting_reason = NULL,
        dependency_key = NULL,
        next_attempt_at = NULL,
        waiting_since = NULL,
        failure_stage = NULL,
        error_message = NULL,
        updated_at = now()
    FROM eligible
    WHERE item.id = eligible.id
    RETURNING item.id, item.task_id, item.attempt_count, item.lease_token,
              item.payload, item.file_id, item.stage, item.input_storage_object_id
), attempts AS (
    INSERT INTO context69.task_attempts (task_id, item_id, attempt, status)
    SELECT task_id, id, attempt_count, 'running'
    FROM claimed
    RETURNING item_id, id AS attempt_id
)
SELECT claimed.id,
       claimed.task_id,
       claimed.attempt_count,
       claimed.lease_token AS "lease_token!",
       claimed.payload,
       claimed.file_id,
       claimed.stage,
       claimed.input_storage_object_id,
       attempts.attempt_id,
       task.kind,
       task.group_id,
       task.group_path,
       task.source_key
FROM claimed
JOIN attempts ON attempts.item_id = claimed.id
JOIN context69.tasks task ON task.id = claimed.task_id
