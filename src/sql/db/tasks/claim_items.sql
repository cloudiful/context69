-- Fast claim path used by the dispatcher on notification-driven wakes.
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
-- Docling polling items (stage = 'docling_poll', waiting_reason = 'external_job')
-- remain claimable when due even at or above the generic five-attempt cap
-- so a live remote conversion can keep polling until its deadline or a
-- terminal/missing-remote resubmit path handles it. The existing
-- next_attempt_at and lease gates still apply and no duplicate submission
-- is permitted; ordinary queued/backoff/dependency items still exhaust at
-- the cap. A due waiting docling_poll external_job item therefore bypasses
-- the generic attempt_count < 5 gate but must still satisfy the due and
-- task-state predicates, and maintenance will not exhaust it either, so a
-- terminal or missing remote job can still be observed and resubmitted
-- through the existing poll code.
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
          OR (
              ti.status = 'waiting'
              AND ti.stage = 'docling_poll'
              AND ti.waiting_reason = 'external_job'
              AND (ti.next_attempt_at IS NULL OR ti.next_attempt_at <= now())
          )
      )
    ORDER BY CASE
                 WHEN ti.waiting_reason = 'external_job' THEN 0
                 ELSE 1
             END,
             ti.created_at
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
