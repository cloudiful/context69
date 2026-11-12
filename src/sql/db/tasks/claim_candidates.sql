-- The items the dispatcher may claim now (issue 1020 P2).
--
-- The selection is the domain policy and nothing else: the lowest ordinal
-- non-terminal item of a parent, the global parent capacity, the created
-- ordering, the retry time, the Docling exclusion and the domain attempt
-- budget. The statement selects the candidates and locks their rows; it never
-- mints a lease, because the lease of an item is the fencing token of the
-- library attempt that holds it.
--
-- `$1` is the global parent-task capacity (`scheduler.max_concurrency`). Every
-- replica passes the same configured capacity and `held` counts the durable
-- lease rows, so the global number of admitted parents stays bounded no matter
-- how many dispatchers run. The cross-replica serialization that makes this
-- broadcast-safe lives in `lock_task_admission.sql`, which the claim
-- transaction runs as its own statement immediately before this one.
--
-- An item whose library segment is still running is not a candidate: the
-- library owns the recovery of an expired segment, and the domain projection
-- of that recovery reopens the item. A candidate whose segment already ended
-- is claimed through a fresh segment, which the claim path produces.
--
-- The returned rows are the pre-claim state of the candidates. `needs_slot`
-- says that the parent takes a new global slot, `parent_lease_until` carries
-- the deadline of the slot the parent already holds.
--
-- A runnable vector rebuild owns admission priority (issue 51 P1). While one
-- is due, no ordinary parent is admitted: an ordinary attempt that is already
-- running finishes first (`drain`), and once it has drained only the single
-- oldest due rebuild may claim. A rebuild that is already running keeps that
-- exclusivity while its live attempt exists, so its own next item and no
-- ordinary item is claimable. A paused, dependency-waiting or backoff rebuild
-- is not runnable and never gates ordinary work.
WITH current_items AS (
    -- The one item of each parent that must finish before any later item of
    -- the same parent can start.
    SELECT DISTINCT ON (item.task_id) item.id, item.task_id, item.ordinal
    FROM context69.task_items item
    WHERE item.status IN ('queued', 'running', 'waiting')
    ORDER BY item.task_id, item.ordinal
),
eligible_items AS (
    SELECT item.id,
           item.task_id,
           item.ordinal,
           ti.execution_task_id,
           parent.kind,
           parent.status AS task_status,
           parent.next_attempt_at AS task_next_attempt_at,
           parent.created_at AS task_created_at,
           parent.deleted_at AS task_deleted_at
    FROM current_items item
    JOIN context69.task_items ti ON ti.id = item.id
    JOIN context69.tasks parent ON parent.id = item.task_id
    WHERE ti.status IN ('queued', 'waiting')
      -- The effective failure usage is attempt_count - paused_attempts < 5;
      -- a parked pause consumes no budget.
      AND (ti.attempt_count - ti.paused_attempts) < 5
      AND parent.pause_state = 'unpaused'
      AND (ti.next_attempt_at IS NULL OR ti.next_attempt_at <= now())
      -- Active remote references survive handoff: the newly leased worker
      -- adopts the recorded identity rather than submitting a replacement.
      -- NULL-safe: only waiting/docling rows are excluded. A plain
      -- `waiting_reason = 'docling'` comparison yields NULL (not true) for
      -- ordinary waiting rows with a NULL reason, and `NOT NULL` would
      -- wrongly filter them out of the dispatcher.
      AND NOT (
            ti.status = 'waiting'
            AND COALESCE(ti.waiting_reason, '') = 'docling'
        )
      -- A paused vector rebuild is awoken by an explicit resume, never by the
      -- dispatcher: the exact dependency key stays unclaimable across a
      -- restart, so a pause cannot turn into an automatic claim loop.
      AND NOT (
            ti.status = 'waiting'
            AND COALESCE(ti.waiting_reason, '') = 'dependency'
            AND ti.dependency_key = 'vector_rebuild_paused'
        )
),
live_running AS (
    -- A parent executes only while one of its items holds a live attempt
    -- lease. The task row status is not enough: a retrying, paused or
    -- dependency-waiting parent keeps its status with no attempt running.
    SELECT item.task_id, parent.kind
    FROM context69.task_items item
    JOIN context69.tasks parent ON parent.id = item.task_id
    WHERE item.status = 'running'
      AND item.lease_until > now()
),
runnable_rebuild AS (
    -- One rebuild eligibility definition shared by every gate below: the
    -- existing candidate policy plus the item execution link the claim path
    -- needs. The oldest due rebuild is the only rebuild a claim may admit.
    SELECT item.task_id
    FROM eligible_items item
    WHERE item.kind = 'vector_rebuild'
      AND item.execution_task_id IS NOT NULL
      AND item.task_status IN ('queued', 'running', 'waiting')
      AND item.task_deleted_at IS NULL
      AND (item.task_next_attempt_at IS NULL OR item.task_next_attempt_at <= now())
    ORDER BY item.task_created_at, item.task_id
    LIMIT 1
),
gate AS (
    -- `normal` keeps the pre-rebuild admission policy. `drain` admits nothing
    -- while a due rebuild waits for the ordinary attempts that are already
    -- running. `rebuild` admits only the one rebuild parent: either the
    -- running rebuild (its next item may still be claimable) or the selected
    -- runnable rebuild.
    SELECT
        CASE
            WHEN running_rebuild_task_id IS NOT NULL THEN 'rebuild'
            WHEN runnable_rebuild_task_id IS NOT NULL AND ordinary_running THEN 'drain'
            WHEN runnable_rebuild_task_id IS NOT NULL THEN 'rebuild'
            ELSE 'normal'
        END AS mode,
        COALESCE(running_rebuild_task_id, runnable_rebuild_task_id) AS rebuild_task_id
    FROM (
        SELECT
            (SELECT task_id
               FROM live_running
              WHERE kind = 'vector_rebuild'
              ORDER BY task_id
              LIMIT 1) AS running_rebuild_task_id,
            EXISTS (SELECT 1 FROM live_running WHERE kind <> 'vector_rebuild')
                AS ordinary_running,
            (SELECT task_id FROM runnable_rebuild) AS runnable_rebuild_task_id
    ) facts
),
work AS (
    -- Parents that need the dispatcher now (a due item) or are already running
    -- one (a live worker item lease). An admitted parent with neither has
    -- nothing to claim and must not have its lease extended. A gated rebuild
    -- narrows both arms to the one rebuild parent; a draining gate admits
    -- nothing at all.
    SELECT item.task_id
    FROM eligible_items item
    CROSS JOIN gate
    WHERE gate.mode = 'normal'
       OR (gate.mode = 'rebuild' AND item.task_id = gate.rebuild_task_id)
    UNION
    SELECT item.task_id
    FROM context69.task_items item
    CROSS JOIN gate
    WHERE item.status = 'running'
      AND item.lease_until > now()
      AND (
          gate.mode = 'normal'
          OR (gate.mode = 'rebuild' AND item.task_id = gate.rebuild_task_id)
      )
),
held AS (
    SELECT count(*)::bigint AS held_count
    FROM context69.tasks task
    WHERE task.lease_token IS NOT NULL
      AND task.lease_until > now()
      AND task.status IN ('queued', 'running', 'waiting')
      AND task.deleted_at IS NULL
),
slot_candidates AS (
    SELECT task.id
    FROM context69.tasks task
    WHERE (task.lease_token IS NULL OR task.lease_until IS NULL OR task.lease_until <= now())
      AND task.status IN ('queued', 'running', 'waiting')
      AND task.deleted_at IS NULL
      AND task.pause_state = 'unpaused'
      AND (task.next_attempt_at IS NULL OR task.next_attempt_at <= now())
      AND task.id IN (SELECT task_id FROM work)
    ORDER BY task.created_at, task.id
    LIMIT GREATEST($1 - (SELECT held_count FROM held), 0)
    FOR UPDATE OF task SKIP LOCKED
),
admitted_work AS (
    -- Parents that already hold a slot and have an item to claim. They are not
    -- limited by capacity and not re-admitted; the slot simply continues. A
    -- gated rebuild narrows this arm to the rebuild parent, and a draining gate
    -- admits nothing.
    SELECT task.id, task.lease_token, task.lease_until
    FROM context69.tasks task
    CROSS JOIN gate
    WHERE task.lease_token IS NOT NULL
      AND task.lease_until > now()
      AND task.status IN ('queued', 'running', 'waiting')
      AND task.deleted_at IS NULL
      AND task.pause_state = 'unpaused'
      AND task.id IN (SELECT task_id FROM eligible_items)
      AND (
          gate.mode = 'normal'
          OR (gate.mode = 'rebuild' AND task.id = gate.rebuild_task_id)
      )
),
claim_targets AS (
    SELECT id FROM slot_candidates
    UNION
    SELECT id FROM admitted_work
),
chosen AS (
    -- One item per claimable parent; `eligible_items` already narrows each
    -- parent to its current item, so the claim advances strictly in order. The
    -- gate is repeated here so a drain or a rebuild-only admission cannot leak
    -- an ordinary item through a target the earlier arms produced.
    SELECT item.id, item.task_id
    FROM eligible_items item
    JOIN claim_targets target ON target.id = item.task_id
    CROSS JOIN gate
    WHERE gate.mode = 'normal'
       OR (gate.mode = 'rebuild' AND item.task_id = gate.rebuild_task_id)
),
locked AS (
    -- The candidate row is locked here, so the claim path holds it until the
    -- transaction ends and no other claimer can take the same item.
    SELECT item.id,
           item.task_id,
           item.execution_task_id,
           item.execution_generation
    FROM context69.task_items item
    JOIN chosen ON chosen.id = item.id
    FOR UPDATE OF item SKIP LOCKED
)
SELECT locked.id,
       locked.task_id,
       locked.execution_task_id,
       locked.execution_generation,
       task.kind,
       task.group_id,
       task.group_path,
       task.source_key,
       EXISTS (SELECT 1 FROM slot_candidates candidate WHERE candidate.id = locked.task_id)
           AS "needs_slot!",
       (SELECT admitted.lease_until
          FROM admitted_work admitted
         WHERE admitted.id = locked.task_id) AS parent_lease_until
FROM locked
JOIN context69.tasks task ON task.id = locked.task_id
