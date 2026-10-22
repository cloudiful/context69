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
WITH current_items AS (
    -- The one item of each parent that must finish before any later item of
    -- the same parent can start.
    SELECT DISTINCT ON (item.task_id) item.id, item.task_id, item.ordinal
    FROM context69.task_items item
    WHERE item.status IN ('queued', 'running', 'waiting')
    ORDER BY item.task_id, item.ordinal
),
eligible_items AS (
    SELECT item.id, item.task_id, item.ordinal
    FROM current_items item
    JOIN context69.task_items ti ON ti.id = item.id
    WHERE ti.status IN ('queued', 'waiting')
      AND ti.attempt_count < 5
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
),
work AS (
    -- Parents that need the dispatcher now (a due item) or are already running
    -- one (a live worker item lease). An admitted parent with neither has
    -- nothing to claim and must not have its lease extended.
    SELECT task_id FROM eligible_items
    UNION
    SELECT item.task_id
    FROM context69.task_items item
    WHERE item.status = 'running'
      AND item.lease_until > now()
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
      AND (task.next_attempt_at IS NULL OR task.next_attempt_at <= now())
      AND task.id IN (SELECT task_id FROM work)
    ORDER BY task.created_at, task.id
    LIMIT GREATEST($1 - (SELECT held_count FROM held), 0)
    FOR UPDATE OF task SKIP LOCKED
),
admitted_work AS (
    -- Parents that already hold a slot and have an item to claim. They are not
    -- limited by capacity and not re-admitted; the slot simply continues.
    SELECT task.id, task.lease_token, task.lease_until
    FROM context69.tasks task
    WHERE task.lease_token IS NOT NULL
      AND task.lease_until > now()
      AND task.status IN ('queued', 'running', 'waiting')
      AND task.deleted_at IS NULL
      AND task.id IN (SELECT task_id FROM eligible_items)
),
claim_targets AS (
    SELECT id FROM slot_candidates
    UNION
    SELECT id FROM admitted_work
),
chosen AS (
    -- One item per claimable parent; `eligible_items` already narrows each
    -- parent to its current item, so the claim advances strictly in order.
    SELECT item.id, item.task_id
    FROM eligible_items item
    JOIN claim_targets target ON target.id = item.task_id
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
