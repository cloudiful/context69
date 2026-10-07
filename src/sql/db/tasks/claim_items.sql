-- Durable parent-task admission and one-item claim (issue 650 P2).
--
-- A parent task owns exactly one global processing slot for its whole
-- lifetime: from admission until it reaches a terminal state. That slot is the
-- durable task lease (`tasks.lease_token`/`tasks.lease_until`). This statement
-- admits at most `$1 - held` new parents, keeps the lease of every admitted
-- parent that still has work, and claims at most one due item per claimable
-- parent. A batch task therefore can never consume several slots, a retry or
-- backoff wait never releases its slot to another parent, and the next item of
-- an admitted parent is claimed inside the same slot.
--
-- `$1` is the global parent-task capacity (`scheduler.max_concurrency`). Every
-- replica passes the same configured capacity and `held` counts the durable
-- lease rows, so the global number of admitted parents stays bounded no matter
-- how many dispatchers run. The capacity check and the grants are one statement
-- so `held` and the admitted rows share a snapshot; the cross-replica
-- serialization that makes that broadcast-safe lives in
-- `lock_task_admission.sql`, which the claim transaction runs as its own
-- statement immediately before this one.
--
-- Lease renewal is deliberately gated on the remaining lease time: a claim wake
-- must not write a task row (and emit a task event) just to extend a lease that
-- is still fresh. Retry waits up to the five-minute backoff budget therefore
-- keep the slot with renewals that happen only when the lease is close to
-- expiring. `maintain_claim_state.sql` renews the same lease while a worker item
-- lease is live (a replica whose local worker pool is full never reaches this
-- statement) and releases it when that item lease is declared expired, and
-- `recompute.sql` clears it once every item is terminal.
--
-- Item gates are unchanged from issue 529 / 639 except the parent projection
-- below: the attempt cap, the expired-lease recovery branch, and the
-- Docling remote-job exclusion that keeps parked or queued items with an
-- active remote row out of the dispatcher until recovery adopts or fences
-- them. A running item whose lease expired is exempt from that exclusion so
-- the reclaim adopts the tracked remote id. Items advance strictly in
-- ordinal order inside the parent slot: only the parent's current (lowest
-- ordinal non-terminal) item is ever eligible, so a parked item blocks its
-- later siblings instead of letting a batch task start several items at once.
-- The crashed worker's attempt is still interrupted inside this statement,
-- scoped to the items being claimed.
--
-- The one parent UPDATE (`parents`) is driven by `claimed`, not by the
-- admission candidates. A candidate whose item row was SKIP LOCKED by a
-- concurrent worker, or whose status guard rejected the claim, therefore
-- never mints or renews a durable task lease: a parent can only hold a
-- concurrency slot once an item of it is actually running in this statement.
-- The same UPDATE writes the item-derived projection (counts, stage, wait
-- fields), so the parent agrees with its items in the claim transaction
-- instead of only after a later `recompute.sql`: the claim flips exactly one
-- item per parent into `running`, so the effective counts are the snapshot
-- counts with that one item moved out of its previous bucket.
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
    WHERE (
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
      -- Crash-recovery adoption (issue 650 P3): a running item whose lease
      -- expired is reclaimed even while its durable remote reference is still
      -- active, so the resumed worker adopts the tracked remote id instead of
      -- recovery cancelling it for a fresh submit. Queued/waiting items with
      -- an active row stay excluded until recovery adopts (legacy parks) or
      -- fences them.
      AND (
            (ti.status = 'running'
                AND (ti.lease_until IS NULL OR ti.lease_until < now()))
            OR NOT EXISTS (
                SELECT 1
                FROM context69.task_docling_remote_jobs remote
                WHERE remote.item_id = ti.id
                  AND remote.status IN ('pending', 'running')
            )
        )
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
    -- The pre-claim status is captured here because a data-modifying CTE's
    -- effects are invisible to the rest of the statement: the parent counts
    -- below are the snapshot counts adjusted by this one status transition.
    SELECT item.id, item.task_id, item.status AS previous_status
    FROM context69.task_items item
    JOIN chosen ON chosen.id = item.id
    FOR UPDATE OF item SKIP LOCKED
),
claimed AS (
    UPDATE context69.task_items AS item
    SET status = 'running',
        attempt_count = item.attempt_count + 1,
        lease_token = gen_random_uuid(),
        lease_until = now() + interval '5 minutes',
        started_at = COALESCE(item.started_at, now()),
        finished_at = NULL,
        waiting_reason = NULL,
        dependency_key = NULL,
        next_attempt_at = NULL,
        waiting_since = NULL,
        failure_stage = NULL,
        error_message = NULL,
        updated_at = now()
    FROM locked
    WHERE item.id = locked.id
      AND item.status IN ('queued', 'waiting', 'running')
      AND (
            item.status <> 'running'
            OR item.lease_until IS NULL
            OR item.lease_until < now()
        )
    RETURNING item.id, item.task_id, item.attempt_count, item.lease_token,
              item.payload, item.file_id, item.stage, item.input_storage_object_id
),
attempts AS (
    INSERT INTO context69.task_attempts (task_id, item_id, attempt, status)
    SELECT task_id, id, attempt_count, 'running'
    FROM claimed
    RETURNING item_id, id AS attempt_id
),
claim_counts AS (
    -- Post-claim item counts for the parents that claimed. A data-modifying
    -- CTE's own effect is invisible to the rest of the statement, so the
    -- snapshot counts are adjusted by hand: every claimed item is moved out
    -- of its snapshot bucket into `running`. A reclaimed expired `running`
    -- item is already in `running` and therefore stays counted exactly once.
    SELECT item.task_id,
           count(*) FILTER (
               WHERE item.status = 'queued'
                 AND NOT (claimed.id IS NOT NULL AND locked.previous_status = 'queued')
           )::bigint AS queued_count,
           count(*) FILTER (
               WHERE item.status = 'running'
                 OR (claimed.id IS NOT NULL AND locked.previous_status IN ('queued', 'waiting'))
           )::bigint AS running_count,
           count(*) FILTER (
               WHERE item.status = 'waiting'
                 AND NOT (claimed.id IS NOT NULL AND locked.previous_status = 'waiting')
           )::bigint AS waiting_count,
           count(*) FILTER (WHERE item.status = 'succeeded')::bigint AS succeeded_count,
           count(*) FILTER (WHERE item.status = 'failed')::bigint AS failed_count,
           count(*) FILTER (WHERE item.status = 'cancelled')::bigint AS cancelled_count,
           -- The claimed item is this parent's current (lowest ordinal
           -- non-terminal) item and the claim never writes `stage`, so its
           -- stage is the parent's current stage. One item per parent is
           -- claimed, so `min` is that single value.
           min(claimed.stage) AS current_stage
    FROM context69.task_items item
    LEFT JOIN locked ON locked.id = item.id
    LEFT JOIN claimed ON claimed.id = item.id
    WHERE item.task_id IN (SELECT task_id FROM claimed)
    GROUP BY item.task_id
),
expired AS (
    UPDATE context69.task_attempts AS attempt
    SET status = 'interrupted',
        failure_stage = 'lease',
        error_message = 'worker lease expired before completion',
        finished_at = now()
    FROM claimed
    JOIN context69.task_items item ON item.id = claimed.id
    WHERE attempt.item_id = claimed.id
      AND item.status = 'running'
      AND (item.lease_until IS NULL OR item.lease_until < now())
      AND attempt.finished_at IS NULL
),
parents AS (
    -- The single parent UPDATE: lease grant/renewal plus the item-derived
    -- projection, both driven by `claim_counts`. Only a parent whose item was
    -- actually claimed is written, so an admission candidate whose item row
    -- was SKIP LOCKED or rejected by the status guard never takes a global
    -- slot. A still-fresh admitted lease keeps its token and deadline; a
    -- slot-less parent mints one. A running item carries no wait reason,
    -- dependency, or retry time, so those clear here exactly as
    -- `recompute.sql` clears them for a running current item.
    UPDATE context69.tasks task
    SET lease_token = CASE
            WHEN candidate.id IS NOT NULL THEN gen_random_uuid()
            ELSE task.lease_token
        END,
        lease_until = CASE
            WHEN candidate.id IS NOT NULL
                 OR admitted.lease_until < now() + interval '4 minutes'
                THEN now() + interval '8 minutes'
            ELSE task.lease_until
        END,
        status = 'running',
        started_at = COALESCE(task.started_at, now()),
        queued_count = counts.queued_count,
        running_count = counts.running_count,
        waiting_count = counts.waiting_count,
        succeeded_count = counts.succeeded_count,
        failed_count = counts.failed_count,
        cancelled_count = counts.cancelled_count,
        stage = counts.current_stage,
        waiting_reason = NULL,
        dependency_key = NULL,
        next_attempt_at = NULL,
        updated_at = now()
    FROM claim_counts counts
    LEFT JOIN slot_candidates candidate ON candidate.id = counts.task_id
    LEFT JOIN admitted_work admitted ON admitted.id = counts.task_id
    WHERE task.id = counts.task_id
      -- Never resurrect a parent that was cancelled or finished after the
      -- candidate CTEs read it.
      AND task.status IN ('queued', 'running', 'waiting')
    RETURNING task.id AS task_id
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
