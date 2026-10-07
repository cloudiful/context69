-- Periodic maintenance for the task-claim hot path.
--
-- The dispatcher runs this only on startup and on the 30-second recovery
-- tick, never on notification-driven fast dispatch. It is idempotent and
-- safe to repeat: exhausted item/file/task propagation only touches rows
-- that already satisfy the existing exhausted predicates
-- (attempt_count >= 5 while queued/waiting), which the fast claim's
-- eligible selection explicitly excludes, and expired-attempt interruption
-- is scoped to abandoned attempts
-- (lease_until IS NULL OR lease_until < now()) so it does not
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
-- This file also owns the two parent-task admission leases (issue 650 P2)
-- that must converge outside the claim path:
--   * `renewed_parent_leases` keeps an admitted parent's slot alive while a
--     worker item lease is live. It is the only renewal that runs when a
--     replica's local worker pool is full, because the dispatcher only reaches
--     its claim statement while a local worker slot is free. Liveness is the
--     item lease, so a crashed owner's task lease is never renewed and expires
--     on its own.
--   * `revoked_parent_leases` releases the slot as soon as the worker item
--     lease that owned it is declared expired, instead of waiting out the full
--     parent lease TTL. Recovery is therefore bounded by the item lease, the
--     same fence that already decides a worker is gone. It also reclaims a
--     lease that no claim can ever serve again (a non-terminal parent with no
--     running item and no claimable or retry-scheduled item), so a slot lost to
--     a crashed claim is freed on this tick rather than after the 8-minute TTL.
-- Both updates target disjoint task rows (an expired running item cannot also
-- be a live running item) and exclude the rows `recomputed` writes, because a
-- task row may only be updated by one CTE per statement.
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
    -- Propagate the exhausted item's failure to its file with the same rule
    -- `project_file_status.sql` uses: never regress a file that already
    -- succeeded, and never steal one that still has an active sibling item.
    -- The guard was `ingest_status = 'failed'`, which restricted the UPDATE to
    -- files that were already failed, so an exhausted item left its file
    -- `running` forever.
    UPDATE context69.library_files AS file
    SET ingest_status = 'failed',
        error_message = 'exceeded maximum attempt count',
        ingested_at = NULL,
        updated_at = now()
    FROM exhausted
    WHERE file.id = exhausted.file_id
      AND exhausted.file_id IS NOT NULL
      AND file.ingest_status <> 'succeeded'
      AND NOT EXISTS (
          SELECT 1
          FROM context69.task_items other
          WHERE other.file_id = file.id
            AND other.id <> exhausted.item_id
            AND other.status IN ('queued', 'running', 'waiting')
      )
    RETURNING file.id
), renewed_parent_leases AS (
    -- An admitted parent keeps its global slot while a worker is running one of
    -- its items. `claim_items.sql` renews the same lease on the claim path, but
    -- a replica whose local worker pool is full never reaches that statement,
    -- so the recovery tick must keep the slot alive here. A task whose only
    -- owner died has no live item lease, so nothing renews it and it expires.
    -- The remaining-lease gate matches the claim path: renewing a still-fresh
    -- lease would write a task row (and emit a task event) on every tick.
    UPDATE context69.tasks AS task
    SET lease_until = now() + interval '8 minutes',
        updated_at = now()
    WHERE task.lease_token IS NOT NULL
      AND task.lease_until > now()
      AND task.lease_until < now() + interval '4 minutes'
      AND task.status IN ('queued', 'running', 'waiting')
      AND task.deleted_at IS NULL
      AND NOT EXISTS (SELECT 1 FROM exhausted e WHERE e.task_id = task.id)
      AND EXISTS (
          SELECT 1
          FROM context69.task_items item
          WHERE item.task_id = task.id
            AND item.status = 'running'
            AND item.lease_until > now()
      )
    RETURNING task.id
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
        -- Head-of-line waiting, identical to `recompute.sql`: the current item
        -- is the lowest-ordinal non-terminal row (the rows exhausted above are
        -- terminal and already excluded), and its status decides the parent.
        status = CASE
            WHEN t.status = 'cancelled'
                 AND counts.queued_count + counts.running_count + counts.waiting_count = 0
                THEN 'cancelled'
            WHEN counts.cancelled_count = t.total_count THEN 'cancelled'
            WHEN counts.succeeded_count + counts.failed_count + counts.cancelled_count = t.total_count
                 AND counts.failed_count = 0 THEN 'succeeded'
            WHEN counts.succeeded_count + counts.failed_count + counts.cancelled_count = t.total_count
                 THEN 'failed'
            WHEN current_item.status IS NULL THEN 'queued'
            ELSE current_item.status
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
        -- Same current-item ordering as `recompute.sql` and `claim_items.sql`:
        -- lowest ordinal non-terminal item, no running/queued/waiting ranking.
        SELECT ti.status, ti.stage, ti.waiting_reason, ti.dependency_key, ti.next_attempt_at
        FROM context69.task_items ti
        WHERE ti.task_id = counts.task_id
          AND ti.status IN ('queued', 'running', 'waiting')
          AND NOT EXISTS (SELECT 1 FROM to_exhaust te WHERE te.id = ti.id)
        ORDER BY ti.ordinal
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
    RETURNING item.id, item.task_id
), revoked_parent_leases AS (
    -- No live worker item lease remains for these tasks: the slot they held is
    -- no longer owned by anything, so release it now instead of letting it idle
    -- until the parent lease TTL runs out. Two shapes qualify. Either an item
    -- lease was declared expired in this statement, or the lease is orphaned
    -- outright: the parent is non-terminal but holds a slot with no running
    -- item and nothing left that any claim could ever serve, so no worker and
    -- no later wake can use it. A parent whose current item is queued (claimable)
    -- or waiting on a scheduled retry legitimately keeps its slot and is never
    -- touched here. Rows already written by `recomputed` are excluded because a
    -- task row may only be updated by one CTE per statement.
    UPDATE context69.tasks AS task
    SET lease_token = NULL,
        lease_until = NULL,
        updated_at = now()
    WHERE task.lease_token IS NOT NULL
      AND task.status IN ('queued', 'running', 'waiting')
      AND task.deleted_at IS NULL
      AND NOT EXISTS (SELECT 1 FROM exhausted e WHERE e.task_id = task.id)
      AND NOT EXISTS (SELECT 1 FROM recomputed r WHERE r.id = task.id)
      AND NOT EXISTS (
          SELECT 1
          FROM context69.task_items item
          WHERE item.task_id = task.id
            AND item.status = 'running'
            AND item.lease_until > now()
            AND NOT EXISTS (SELECT 1 FROM expired_items revoked WHERE revoked.id = item.id)
      )
      AND (
          EXISTS (SELECT 1 FROM expired_items revoked WHERE revoked.task_id = task.id)
          OR NOT EXISTS (
              SELECT 1
              FROM context69.task_items item
              WHERE item.task_id = task.id
                AND item.status IN ('queued', 'running', 'waiting')
                AND item.attempt_count < 5
          )
      )
    RETURNING task.id
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
    (SELECT count(*) FROM recomputed) AS "exhausted_tasks!",
    (SELECT count(*) FROM expired) AS "expired_attempts!",
    (SELECT count(*) FROM renewed_parent_leases) AS "renewed_parent_leases!",
    (SELECT count(*) FROM revoked_parent_leases) AS "revoked_parent_leases!"
