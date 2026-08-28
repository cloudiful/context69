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
--
-- Parent aggregates are recomputed atomically from the post-exhaustion
-- effective item state. PostgreSQL data-modifying CTEs share a snapshot,
-- so a later CTE cannot see the earlier UPDATE's row changes via a plain
-- read of the target table, and the same parent row must not be updated
-- from two CTEs in one statement. This file therefore uses a single
-- authoritative parent UPDATE: to_exhaust captures the candidate rows and
-- their pre-update state, exhausted updates them, and the parent
-- recompute derives effective counts/status from the snapshot plus the
-- captured to_exhaust rows.
--
-- External-job lifecycle reconciliation is conservative and idempotent.
-- Any local pending/running task_external_jobs rows attached to terminal
-- task_items (succeeded, failed, cancelled) — including items newly
-- exhausted by this statement — are locally moved to cancelled with an
-- explicit reason that remote cancellation was not requested. The check
-- uses the statement's snapshot terminal items plus the captured
-- to_exhaust ids so the transition does not require a second maintenance
-- call. `submitting` rows are never touched because the remote submission
-- outcome is uncertain and must remain manual-recovery-required; active
-- items (queued, running, waiting) are left alone. No external request
-- is made and the same external-job row is never updated from two CTEs
-- in this statement.
WITH to_exhaust AS (
    SELECT item.id, item.task_id, item.file_id, item.status AS old_status, item.ordinal
    FROM context69.task_items AS item
    JOIN context69.tasks AS task ON task.id = item.task_id
    WHERE (
            task.status IN ('queued', 'running')
            OR (
                task.status = 'waiting'
                AND (task.next_attempt_at IS NULL OR task.next_attempt_at <= now())
            )
        )
      AND item.status IN ('queued', 'waiting')
      AND item.attempt_count >= 5
      AND (item.next_attempt_at IS NULL OR item.next_attempt_at <= now())
    FOR UPDATE OF item
), exhausted AS (
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
    FROM to_exhaust
    WHERE item.id = to_exhaust.id
    RETURNING item.task_id, item.id AS item_id, item.file_id, to_exhaust.old_status, to_exhaust.ordinal
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
), recomputed AS (
    UPDATE context69.tasks t
    SET queued_count = counts.queued_count,
        running_count = counts.running_count,
        waiting_count = counts.waiting_count,
        succeeded_count = counts.succeeded_count,
        failed_count = counts.failed_count,
        cancelled_count = counts.cancelled_count,
        failure_stage = (
            SELECT failure_stage FROM (
                SELECT ti.failure_stage, ti.ordinal
                FROM context69.task_items ti
                WHERE ti.task_id = t.id
                  AND ti.status = 'failed'
                  AND ti.failure_stage IS NOT NULL
                  AND NOT EXISTS (SELECT 1 FROM to_exhaust te WHERE te.id = ti.id)
                UNION ALL
                SELECT 'attempts'::text AS failure_stage, te.ordinal
                FROM to_exhaust te
                WHERE te.task_id = t.id
            ) u
            ORDER BY ordinal
            LIMIT 1
        ),
        error_summary = (
            SELECT error_message FROM (
                SELECT ti.error_message, ti.ordinal
                FROM context69.task_items ti
                WHERE ti.task_id = t.id
                  AND ti.status = 'failed'
                  AND ti.error_message IS NOT NULL
                  AND NOT EXISTS (SELECT 1 FROM to_exhaust te WHERE te.id = ti.id)
                UNION ALL
                SELECT 'exceeded maximum attempt count'::text AS error_message, te.ordinal
                FROM to_exhaust te
                WHERE te.task_id = t.id
            ) u
            ORDER BY ordinal
            LIMIT 1
        ),
        stage = current_item.stage,
        waiting_reason = current_item.waiting_reason,
        dependency_key = current_item.dependency_key,
        next_attempt_at = current_item.next_attempt_at,
        lease_token = CASE WHEN counts.succeeded_count + counts.failed_count + counts.cancelled_count = t.total_count THEN NULL ELSE t.lease_token END,
        lease_until = CASE WHEN counts.succeeded_count + counts.failed_count + counts.cancelled_count = t.total_count THEN NULL ELSE t.lease_until END,
        status = CASE
            WHEN t.status = 'cancelled'
                 AND counts.queued_count + counts.running_count + counts.waiting_count = 0
                THEN 'cancelled'
            WHEN counts.cancelled_count = t.total_count THEN 'cancelled'
            WHEN counts.succeeded_count + counts.failed_count + counts.cancelled_count = t.total_count
                 AND counts.failed_count = 0 THEN 'succeeded'
            WHEN counts.succeeded_count + counts.failed_count + counts.cancelled_count = t.total_count
                 THEN 'failed'
            WHEN counts.running_count > 0 THEN 'running'
            WHEN counts.queued_count > 0 THEN 'queued'
            WHEN counts.waiting_count > 0 THEN 'waiting'
            ELSE 'queued'
        END,
        finished_at = CASE
            WHEN counts.succeeded_count + counts.failed_count + counts.cancelled_count = t.total_count
            THEN now()
            ELSE NULL
        END,
        updated_at = now()
    FROM (
        SELECT
            agg.task_id,
            (SELECT count(*)::bigint FROM context69.task_items ti WHERE ti.task_id = agg.task_id AND ti.status = 'queued' AND NOT EXISTS (SELECT 1 FROM to_exhaust te WHERE te.id = ti.id)) AS queued_count,
            (SELECT count(*)::bigint FROM context69.task_items ti WHERE ti.task_id = agg.task_id AND ti.status = 'running' AND NOT EXISTS (SELECT 1 FROM to_exhaust te WHERE te.id = ti.id)) AS running_count,
            (SELECT count(*)::bigint FROM context69.task_items ti WHERE ti.task_id = agg.task_id AND ti.status = 'waiting' AND NOT EXISTS (SELECT 1 FROM to_exhaust te WHERE te.id = ti.id)) AS waiting_count,
            (SELECT count(*)::bigint FROM context69.task_items ti WHERE ti.task_id = agg.task_id AND ti.status = 'succeeded' AND NOT EXISTS (SELECT 1 FROM to_exhaust te WHERE te.id = ti.id)) AS succeeded_count,
            (SELECT count(*)::bigint FROM context69.task_items ti WHERE ti.task_id = agg.task_id AND ti.status = 'failed' AND NOT EXISTS (SELECT 1 FROM to_exhaust te WHERE te.id = ti.id)) + (SELECT count(*)::bigint FROM to_exhaust te WHERE te.task_id = agg.task_id) AS failed_count,
            (SELECT count(*)::bigint FROM context69.task_items ti WHERE ti.task_id = agg.task_id AND ti.status = 'cancelled' AND NOT EXISTS (SELECT 1 FROM to_exhaust te WHERE te.id = ti.id)) AS cancelled_count
        FROM (SELECT DISTINCT task_id FROM to_exhaust) agg
    ) counts
    LEFT JOIN LATERAL (
        SELECT stage, waiting_reason, dependency_key, next_attempt_at
        FROM (
            SELECT ti.stage, ti.waiting_reason, ti.dependency_key, ti.next_attempt_at, ti.ordinal,
                   CASE ti.status WHEN 'queued' THEN 0 WHEN 'running' THEN 1 ELSE 2 END AS prio
            FROM context69.task_items ti
            WHERE ti.task_id = counts.task_id
              AND ti.status IN ('queued', 'running', 'waiting')
              AND NOT EXISTS (SELECT 1 FROM to_exhaust te WHERE te.id = ti.id)
        ) sub
        ORDER BY prio, next_attempt_at NULLS FIRST, ordinal
        LIMIT 1
    ) current_item ON TRUE
    WHERE t.id = counts.task_id
    RETURNING t.id
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
), reconciled_external_jobs AS (
    UPDATE context69.task_external_jobs AS job
    SET status = 'cancelled',
        remote_status = COALESCE(job.remote_status, job.status),
        error_message = COALESCE(
            job.error_message,
            'task item is terminal; local external job cancelled without remote cancellation'
        ),
        last_polled_at = now(),
        updated_at = now()
    WHERE job.status IN ('pending', 'running')
      AND (
          EXISTS (
              SELECT 1
              FROM context69.task_items ti
              WHERE ti.id = job.item_id
                AND ti.status IN ('succeeded', 'failed', 'cancelled')
          )
          OR EXISTS (
              SELECT 1 FROM to_exhaust te WHERE te.id = job.item_id
          )
      )
    RETURNING job.id
)
SELECT
    (SELECT count(*) FROM exhausted) AS "exhausted_items!",
    (SELECT count(*) FROM exhausted_files) AS "exhausted_files!",
    (SELECT count(*) FROM recomputed) AS "exhausted_tasks!",
    (SELECT count(*) FROM expired) AS "expired_attempts!"
