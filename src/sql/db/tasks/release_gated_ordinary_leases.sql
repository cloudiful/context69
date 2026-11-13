-- Releases idle parent admission leases that a rebuild gate or a parked rebuild
-- makes pure capacity pressure (issue 51 P1/P2).
--
-- A normal parent keeps its admission slot for up to eight minutes after its
-- last attempt ends. That idle slot is pure capacity pressure, so the slots
-- that no running attempt owns are released before candidate selection, inside
-- the same admission lock as the claim. Only the lease authority is cleared: no
-- item, attempt, task status, or terminal row is touched, so an attempt that is
-- still running is never interrupted.
--
-- Two scopes release:
--   * an ordinary parent, only while a live rebuild attempt or a runnable
--     rebuild gates admission, so normal ordinary lease reuse is unchanged;
--   * a rebuild parent that is not the selected runnable rebuild, regardless of
--     whether any rebuild is runnable, so a parked rebuild (explicit pause,
--     external wait, retry backoff, missing execution link or exhausted budget)
--     does not hold the slot until its TTL. The selected runnable rebuild keeps
--     its slot, and a live attempt is always protected.
--
-- The rebuild gate and oldest selection mirror `claim_candidates.sql`: the same
-- eligibility predicates and the same `created_at, task_id` ordering decide
-- which rebuild is selected.
WITH current_items AS (
    SELECT DISTINCT ON (item.task_id)
           item.id,
           item.task_id,
           item.ordinal,
           item.status,
           item.attempt_count,
           item.paused_attempts,
           item.next_attempt_at,
           item.waiting_reason,
           item.dependency_key,
           item.execution_task_id,
           parent.kind,
           parent.status AS task_status,
           parent.pause_state,
           parent.next_attempt_at AS task_next_attempt_at,
           parent.created_at AS task_created_at,
           parent.deleted_at AS task_deleted_at
    FROM context69.task_items item
    JOIN context69.tasks parent ON parent.id = item.task_id
    WHERE item.status IN ('queued', 'running', 'waiting')
      AND parent.kind = 'vector_rebuild'
    ORDER BY item.task_id, item.ordinal
),
runnable_rebuild AS (
    SELECT item.task_id
    FROM current_items item
    WHERE item.status IN ('queued', 'waiting')
      AND (item.attempt_count - item.paused_attempts) < 5
      AND item.pause_state = 'unpaused'
      AND item.task_status IN ('queued', 'running', 'waiting')
      AND item.task_deleted_at IS NULL
      AND item.execution_task_id IS NOT NULL
      AND (item.next_attempt_at IS NULL OR item.next_attempt_at <= now())
      AND (
          item.task_next_attempt_at IS NULL
          OR item.task_next_attempt_at <= now()
      )
      AND NOT (
          item.status = 'waiting'
          AND COALESCE(item.waiting_reason, '') = 'docling'
      )
      AND NOT (
          item.status = 'waiting'
          AND COALESCE(item.waiting_reason, '') = 'dependency'
          AND item.dependency_key = 'vector_rebuild_paused'
      )
    ORDER BY item.task_created_at, item.task_id
    LIMIT 1
),
live_running AS (
    SELECT parent.kind
    FROM context69.task_items item
    JOIN context69.tasks parent ON parent.id = item.task_id
    WHERE item.status = 'running'
      AND EXISTS (
          SELECT 1
          FROM cloudiful_tasks.attempts attempt
          JOIN cloudiful_tasks.tasks library_task
            ON library_task.id = attempt.task_id
          WHERE attempt.task_id = item.execution_task_id
            AND attempt.fencing_token = item.lease_token
            AND attempt.status = 'running'
            AND attempt.lease_expires_at > clock_timestamp()
            AND library_task.namespace = 'context69'
            AND library_task.queue = 'items'
            AND library_task.kind = 'context69.item'
      )
),
gate AS (
    SELECT
        EXISTS (SELECT 1 FROM live_running WHERE kind = 'vector_rebuild')
        OR EXISTS (SELECT 1 FROM runnable_rebuild) AS rebuild_gates,
        (SELECT task_id FROM runnable_rebuild) AS selected_rebuild_task_id
),
release_scope AS (
    SELECT task.id
    FROM context69.tasks task
    CROSS JOIN gate
    WHERE task.lease_token IS NOT NULL
      AND task.lease_until > now()
      AND task.status IN ('queued', 'running', 'waiting')
      AND task.deleted_at IS NULL
      AND NOT EXISTS (
          SELECT 1
          FROM context69.task_items item
          WHERE item.task_id = task.id
            AND item.status = 'running'
            AND EXISTS (
                SELECT 1
                FROM cloudiful_tasks.attempts attempt
                JOIN cloudiful_tasks.tasks library_task
                  ON library_task.id = attempt.task_id
                WHERE attempt.task_id = item.execution_task_id
                  AND attempt.fencing_token = item.lease_token
                  AND attempt.status = 'running'
                  AND attempt.lease_expires_at > clock_timestamp()
                  AND library_task.namespace = 'context69'
                  AND library_task.queue = 'items'
                  AND library_task.kind = 'context69.item'
            )
      )
      AND (
          (task.kind <> 'vector_rebuild' AND gate.rebuild_gates)
          OR (
              task.kind = 'vector_rebuild'
              AND task.id IS DISTINCT FROM gate.selected_rebuild_task_id
          )
      )
),
released AS (
    UPDATE context69.tasks task
    SET lease_token = NULL,
        lease_until = NULL,
        updated_at = now()
    FROM release_scope scope
    WHERE task.id = scope.id
    RETURNING task.id
)
SELECT count(*)::bigint AS "released_parent_leases!" FROM released
