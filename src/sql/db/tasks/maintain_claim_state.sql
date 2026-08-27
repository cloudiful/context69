-- Periodic maintenance for the task-claim hot path.
--
-- The dispatcher runs this only on startup and on the 30-second recovery
-- tick, never on notification-driven fast dispatch. It is idempotent and
-- safe to repeat: exhausted item/file/task propagation only touches rows
-- that already satisfy the existing exhausted predicates
-- (attempt_count >= 5 while queued/waiting), which the fast claim's
-- eligible selection explicitly excludes, and expired-attempt interruption
-- is scoped to abandoned attempts (lease_until IS NULL OR lease_until < now())
-- so it does not
-- clear active leases. When an expired running item is interrupted, its
-- item lease (lease_token/lease_until) is atomically revoked in the same
-- statement so a late worker holding the old token cannot
-- finish/heartbeat/progress; the item stays running so the fast claim
-- path can reclaim it. The dispatcher runs maintenance sequentially before
-- the recovery dispatch so the two statements do not race for the same
-- attempt rows inside one recovery cycle, but concurrent callers remain
-- safe to retry because all predicates remain valid if run again. The
-- exhausted CTEs match the predicates that previously lived inside
-- claim_items.sql; splitting them out lets the fast claim path skip the
-- maintenance UPDATE/RETURNING work when the queue is empty and lets the
-- recovery tick keep converging exhausted-only queues toward terminal
-- state.
WITH exhausted AS (
    UPDATE context69.task_items AS item
    SET status = 'failed',
        failure_stage = 'attempts',
        error_message = 'exceeded maximum attempt count',
        lease_token = NULL,
        lease_until = NULL,
        waiting_reason = NULL,
        dependency_key = NULL,
        next_attempt_at = NULL,
        finished_at = now(),
        updated_at = now()
    FROM context69.tasks AS task
    WHERE item.task_id = task.id
      AND (
          task.status IN ('queued', 'running')
          OR (
              task.status = 'waiting'
              AND (task.next_attempt_at IS NULL OR task.next_attempt_at <= now())
          )
      )
      AND item.status IN ('queued', 'waiting')
      AND item.attempt_count >= 5
      AND (item.next_attempt_at IS NULL OR item.next_attempt_at <= now())
    RETURNING item.task_id, item.id AS item_id, item.file_id
), exhausted_files AS (
    UPDATE context69.library_files AS file
    SET ingest_status = 'failed',
        error_message = 'exceeded maximum attempt count',
        updated_at = now()
    FROM exhausted
    WHERE file.id = exhausted.file_id
      AND exhausted.file_id IS NOT NULL
      AND file.ingest_status IN ('pending', 'running')
      AND NOT EXISTS (
          SELECT 1
          FROM context69.task_items other
          WHERE other.file_id = file.id
            AND other.id <> exhausted.item_id
            AND other.status IN ('queued', 'running', 'waiting')
      )
    RETURNING file.id
), exhausted_tasks AS (
    UPDATE context69.tasks AS task
    SET status = 'failed',
        failure_stage = 'attempts',
        error_summary = 'task items exceeded the maximum attempt count',
        finished_at = now(),
        updated_at = now()
    FROM exhausted
    WHERE task.id = exhausted.task_id
      AND NOT EXISTS (
          SELECT 1
          FROM context69.task_items ti
          WHERE ti.task_id = task.id
            AND ti.status IN ('queued', 'waiting')
            AND ti.attempt_count < 5
      )
      AND NOT EXISTS (
          SELECT 1
          FROM context69.task_items ti
          WHERE ti.task_id = task.id
            AND ti.status = 'running'
      )
    RETURNING task.id
), expired_items AS (
    UPDATE context69.task_items AS item
    SET lease_token = NULL,
        lease_until = NULL,
        updated_at = now()
    FROM context69.tasks AS task
    WHERE item.task_id = task.id
      AND item.status = 'running'
      AND (item.lease_until IS NULL OR item.lease_until < now())
      AND (
          task.status IN ('queued', 'running')
          OR (
              task.status = 'waiting'
              AND (task.next_attempt_at IS NULL OR task.next_attempt_at <= now())
          )
      )
    RETURNING item.id
), expired AS (
    UPDATE context69.task_attempts AS attempt
    SET status = 'interrupted',
        failure_stage = 'lease',
        error_message = 'worker lease expired before completion',
        finished_at = now()
    FROM expired_items
    WHERE attempt.item_id = expired_items.id
      AND attempt.finished_at IS NULL
    RETURNING attempt.id
)
SELECT
    (SELECT count(*) FROM exhausted) AS "exhausted_items!",
    (SELECT count(*) FROM exhausted_files) AS "exhausted_files!",
    (SELECT count(*) FROM exhausted_tasks) AS "exhausted_tasks!",
    (SELECT count(*) FROM expired) AS "expired_attempts!"
