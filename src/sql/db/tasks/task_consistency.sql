-- Parent/item consistency snapshot (issue 702 P3).
--
-- `task_items` is the execution-state source of truth and every parent counter
-- and summary field is its projection, so this statement compares the two and
-- reports the disagreement instead of rewriting anything. It is read-only and
-- never mutates a row: repair stays in the transitions themselves
-- (`recompute.sql`, `claim_items.sql`, `maintain_claim_state.sql`).
--
-- `$1` scopes the statement to one task. `/healthz` passes NULL to get the
-- whole-queue gauges; `GET /v1/tasks/{task_id}/diagnose` passes the task id to
-- get that task's verdict plus its per-item attempt and lease forensics. Both
-- consumers therefore share one consistency definition and one current-item
-- ordering: the lowest-ordinal non-terminal item, exactly like
-- `recompute.sql` and `claim_items.sql`. The per-item diagnostics branch is
-- guarded on the scope filter, so the health snapshot never pays for it.
WITH scoped_parent AS (
    SELECT task.id,
           task.status,
           task.total_count,
           task.queued_count,
           task.running_count,
           task.waiting_count,
           task.succeeded_count,
           task.failed_count,
           task.cancelled_count,
           task.stage,
           task.waiting_reason,
           task.dependency_key,
           task.lease_until,
           task.started_at
    FROM context69.tasks task
    WHERE ($1::uuid IS NULL OR task.id = $1::uuid)
      AND task.deleted_at IS NULL
),
item_counts AS (
    -- One grouped pass per scoped parent. A parent with no items has no row
    -- here; every read below coalesces, so "created but not yet fanned out"
    -- reports its own mismatch instead of silently passing.
    SELECT item.task_id,
           count(*)::bigint AS item_count,
           count(*) FILTER (WHERE item.status = 'queued')::bigint AS queued_count,
           count(*) FILTER (WHERE item.status = 'running')::bigint AS running_count,
           count(*) FILTER (WHERE item.status = 'waiting')::bigint AS waiting_count,
           count(*) FILTER (WHERE item.status = 'succeeded')::bigint AS succeeded_count,
           count(*) FILTER (WHERE item.status = 'failed')::bigint AS failed_count,
           count(*) FILTER (WHERE item.status = 'cancelled')::bigint AS cancelled_count,
           -- A parent slot is only legitimately held while one of its item
           -- leases is still live, so the liveness test belongs here rather
           -- than in a per-parent correlated subquery.
           count(*) FILTER (
               WHERE item.status = 'running'
                 AND (item.lease_until IS NULL OR item.lease_until > now())
           )::bigint AS live_running_count,
           count(*) FILTER (
               WHERE item.status IN ('queued', 'running', 'waiting')
                 AND item.attempt_count >= 4
           )::bigint AS near_exhaustion_count,
           -- Whether any item of this parent could still be served by a claim.
           -- `claim_items.sql` excludes `attempt_count >= 5`, so this is the
           -- exact eligibility shape the P1 reclaim predicate tests.
           EXISTS (
               SELECT 1
               FROM context69.task_items candidate
               WHERE candidate.task_id = item.task_id
                 AND candidate.status IN ('queued', 'running', 'waiting')
                 AND candidate.attempt_count < 5
           ) AS claimable_item_exists,
           -- A `running` item whose lease has already lapsed: the durable
           -- evidence that its worker is gone, which is one of the two shapes
           -- `maintain_claim_state.sql` reclaims a parent lease for.
           EXISTS (
               SELECT 1
               FROM context69.task_items lapsed
               WHERE lapsed.task_id = item.task_id
                 AND lapsed.status = 'running'
                 AND (lapsed.lease_until IS NULL OR lapsed.lease_until <= now())
           ) AS expired_running_item_exists
    FROM context69.task_items item
    WHERE item.task_id IN (SELECT parent.id FROM scoped_parent parent)
    GROUP BY item.task_id
),
current_item AS (
    SELECT DISTINCT ON (item.task_id) item.task_id,
           item.id,
           item.status,
           item.stage,
           item.waiting_reason,
           item.dependency_key
    FROM context69.task_items item
    WHERE item.task_id IN (SELECT parent.id FROM scoped_parent parent)
      AND item.status IN ('queued', 'running', 'waiting')
    ORDER BY item.task_id, item.ordinal
),
open_attempts AS (
    SELECT count(*)::bigint AS open_attempt_count
    FROM context69.task_attempts attempt
    WHERE attempt.task_id IN (SELECT parent.id FROM scoped_parent parent)
      AND attempt.finished_at IS NULL
),
item_diagnostics AS (
    -- Per-item lease deadline plus attempt forensics. Guarded on the scope
    -- filter, which is a constant: the whole branch is skipped for the
    -- whole-queue health snapshot instead of aggregating every item.
    SELECT item.ordinal,
           jsonb_build_object(
               'item_id', item.id,
               'ordinal', item.ordinal,
               'lease_expires_at', item.lease_until,
               'active_attempt', active.attempt,
               'latest_attempt', latest.attempt
           ) AS entry
    FROM context69.task_items item
    LEFT JOIN LATERAL (
        SELECT jsonb_build_object(
                   'attempt_id', attempt.id,
                   'attempt', attempt.attempt,
                   'status', attempt.status,
                   'retryable', attempt.retryable,
                   'failure_stage', attempt.failure_stage,
                   'error_message', attempt.error_message,
                   'started_at', attempt.started_at,
                   'finished_at', attempt.finished_at
               ) AS attempt
        FROM context69.task_attempts attempt
        WHERE attempt.item_id = item.id
          AND attempt.finished_at IS NULL
        ORDER BY attempt.id DESC
        LIMIT 1
    ) active ON $1::uuid IS NOT NULL
    LEFT JOIN LATERAL (
        SELECT jsonb_build_object(
                   'attempt_id', attempt.id,
                   'attempt', attempt.attempt,
                   'status', attempt.status,
                   'retryable', attempt.retryable,
                   'failure_stage', attempt.failure_stage,
                   'error_message', attempt.error_message,
                   'started_at', attempt.started_at,
                   'finished_at', attempt.finished_at
               ) AS attempt
        FROM context69.task_attempts attempt
        WHERE attempt.item_id = item.id
        ORDER BY attempt.id DESC
        LIMIT 1
    ) latest ON $1::uuid IS NOT NULL
    WHERE item.task_id IN (SELECT parent.id FROM scoped_parent parent)
      AND $1::uuid IS NOT NULL
),
parent_facts AS (
    -- One row of derived facts per scoped parent. `verdict` below turns these
    -- into the mismatch list, so the item-derived status is computed once here
    -- instead of twice inside a single SELECT.
    SELECT parent.id AS task_id,
           parent.status AS parent_status,
           parent.status IN ('queued', 'running', 'waiting') AS is_active,
           parent.lease_until,
           parent.started_at,
           parent.total_count,
           parent.queued_count AS parent_queued_count,
           parent.running_count AS parent_running_count,
           parent.waiting_count AS parent_waiting_count,
           parent.succeeded_count AS parent_succeeded_count,
           parent.failed_count AS parent_failed_count,
           parent.cancelled_count AS parent_cancelled_count,
           parent.stage AS parent_stage,
           parent.waiting_reason AS parent_waiting_reason,
           parent.dependency_key AS parent_dependency_key,
           current.id AS current_item_id,
           current.stage AS current_stage,
           current.status AS current_status,
           current.waiting_reason AS current_waiting_reason,
           current.dependency_key AS current_dependency_key,
           COALESCE(counts.item_count, 0) AS item_count,
           COALESCE(counts.queued_count, 0) AS queued_count,
           COALESCE(counts.running_count, 0) AS running_count,
           COALESCE(counts.waiting_count, 0) AS waiting_count,
           COALESCE(counts.succeeded_count, 0) AS succeeded_count,
           COALESCE(counts.failed_count, 0) AS failed_count,
           COALESCE(counts.cancelled_count, 0) AS cancelled_count,
           COALESCE(counts.live_running_count, 0) AS live_running_count,
           COALESCE(counts.near_exhaustion_count, 0) AS near_exhaustion_count,
           COALESCE(counts.claimable_item_exists, false) AS claimable_item_exists,
           COALESCE(counts.expired_running_item_exists, false)
               AS expired_running_item_exists,
           -- The status the items imply, derived with exactly the CASE that
           -- `recompute.sql` writes to the parent. A parent whose status
           -- disagrees here is stale even when every counter matches, so the
           -- verdict compares the two instead of trusting the parent.
           CASE
               WHEN parent.status = 'cancelled'
                    AND COALESCE(counts.queued_count, 0)
                        + COALESCE(counts.running_count, 0)
                        + COALESCE(counts.waiting_count, 0) = 0
                   THEN 'cancelled'
               WHEN COALESCE(counts.cancelled_count, 0) = parent.total_count
                   THEN 'cancelled'
               WHEN COALESCE(counts.succeeded_count, 0)
                    + COALESCE(counts.failed_count, 0)
                    + COALESCE(counts.cancelled_count, 0) = parent.total_count
                    AND COALESCE(counts.failed_count, 0) = 0
                   THEN 'succeeded'
               WHEN COALESCE(counts.succeeded_count, 0)
                    + COALESCE(counts.failed_count, 0)
                    + COALESCE(counts.cancelled_count, 0) = parent.total_count
                   THEN 'failed'
               WHEN current.status IS NULL THEN 'queued'
               ELSE current.status
           END AS derived_status
    FROM scoped_parent parent
    LEFT JOIN item_counts counts ON counts.task_id = parent.id
    LEFT JOIN current_item current ON current.task_id = parent.id
),
verdict AS (
    -- The mismatching fields, named so an operator sees the breach instead of
    -- a bare boolean. Wait fields compare with NULL-safe equality: an absent
    -- field and an empty-string field are the same fact.
    --
    -- `status` is compared against `derived_status`, so a stale terminal or
    -- running status is reported even when every counter matches: counting
    -- agreement alone would call a parent consistent while its own status
    -- column contradicts its items.
    --
    -- `stage` is compared only for a parent that has left `queued`. Every other
    -- parent summary field is a pure projection of the items, but `create.sql`
    -- seeds a queued parent's stage with the kind's entry stage so a task that
    -- has not been admitted yet still reports the work it is about to do.
    -- Comparing that seed against the collapsed `processing` item column would
    -- flag every freshly submitted task as inconsistent. Once the parent is
    -- running or waiting, the claim and recompute transitions own the column
    -- and any divergence is a real breach. Both values stay visible in the
    -- diagnose response either way.
    SELECT fact.*,
           array_remove(
               ARRAY[
                   CASE WHEN fact.parent_status <> fact.derived_status
                       THEN 'status' END,
                   CASE WHEN fact.item_count <> fact.total_count
                       THEN 'total_count' END,
                   CASE WHEN fact.queued_count <> fact.parent_queued_count
                       THEN 'queued_count' END,
                   CASE WHEN fact.running_count <> fact.parent_running_count
                       THEN 'running_count' END,
                   CASE WHEN fact.waiting_count <> fact.parent_waiting_count
                       THEN 'waiting_count' END,
                   CASE WHEN fact.succeeded_count <> fact.parent_succeeded_count
                       THEN 'succeeded_count' END,
                   CASE WHEN fact.failed_count <> fact.parent_failed_count
                       THEN 'failed_count' END,
                   CASE WHEN fact.cancelled_count <> fact.parent_cancelled_count
                       THEN 'cancelled_count' END,
                   CASE WHEN fact.parent_status <> 'queued'
                             AND COALESCE(fact.parent_stage, '')
                                 IS DISTINCT FROM COALESCE(fact.current_stage, '')
                       THEN 'stage' END,
                   CASE WHEN COALESCE(fact.parent_waiting_reason, '')
                             IS DISTINCT FROM COALESCE(fact.current_waiting_reason, '')
                       THEN 'waiting_reason' END,
                   CASE WHEN COALESCE(fact.parent_dependency_key, '')
                             IS DISTINCT FROM COALESCE(fact.current_dependency_key, '')
                       THEN 'dependency_key' END
               ],
               NULL
           ) AS mismatch_fields
    FROM parent_facts fact
)
SELECT
    (SELECT count(*) FROM verdict)::bigint AS "parent_count!",
    (SELECT count(*) FROM verdict WHERE is_active)::bigint AS "active_parent_count!",
    (
        SELECT count(*) FROM verdict
        WHERE current_waiting_reason = 'dependency'
    )::bigint AS "dependency_waiting_parent_count!",
    COALESCE(
        (SELECT jsonb_agg(jsonb_build_object('key', status.key, 'count', status.count)
                         ORDER BY status.key)
         FROM (
             SELECT parent_status AS key, count(*)::bigint AS count
             FROM verdict
             GROUP BY parent_status
         ) status),
        '[]'::jsonb
    ) AS parent_status_counts,
    (
        SELECT count(*) FROM verdict
        WHERE parent_status = 'running' AND running_count = 0
    )::bigint AS "running_parent_without_running_item_count!",
    (SELECT COALESCE(sum(live_running_count), 0)::bigint FROM verdict)::bigint
        AS "live_running_count!",
    -- An orphaned parent slot: a live admission lease with no live worker item
    -- lease, and nothing a future claim could ever serve. This mirrors the
    -- reclaim predicate of `maintain_claim_state.sql` exactly, so the gauge
    -- never counts a legitimate wait:
    --   * a parent whose current item is claimable (queued/running/waiting with
    --     `attempt_count < 5`, which is what `claim_items.sql` admits) keeps its
    --     slot across a scheduled retry or dependency wait, and is not an
    --     orphan;
    --   * a parent whose only `running` item has a lapsed lease is the crashed
    --     -worker shape maintenance reclaims on its next tick.
    -- Reading `lease_until` alone would flag every backoff or dependency wait
    -- as an orphan and drown the real signal.
    (
        SELECT count(*) FROM verdict
        WHERE is_active
          AND lease_until IS NOT NULL
          AND lease_until > now()
          AND live_running_count = 0
          AND (expired_running_item_exists OR NOT claimable_item_exists)
    )::bigint AS "lease_without_running_item_count!",
    (SELECT open_attempt_count FROM open_attempts)::bigint AS "open_attempt_count!",
    (
        SELECT COALESCE(sum(near_exhaustion_count), 0)::bigint FROM verdict
    )::bigint AS "near_exhaustion_item_count!",
    -- Age since admission, not the remaining lease. `started_at` is stamped
    -- when the claim admits the parent (`started_at = COALESCE(started_at,
    -- now())`), so it is the earliest timestamp that means "this parent has
    -- held a slot since". `lease_until` is a future deadline, so measuring
    -- against it would clamp every value to zero.
    (
        SELECT min(started_at) FROM verdict
        WHERE is_active AND lease_until IS NOT NULL AND lease_until > now()
    ) AS oldest_admitted_at,
    (
        SELECT count(*) FROM verdict WHERE cardinality(mismatch_fields) > 0
    )::bigint AS "parent_item_mismatch_count!",
    (SELECT current_item_id FROM verdict ORDER BY task_id LIMIT 1) AS current_item_id,
    -- Single-task scope only: the whole-queue scope has many parents, so this
    -- column is meaningful exactly when `$1` names one task.
    (SELECT lease_until FROM verdict ORDER BY task_id LIMIT 1) AS scoped_lease_until,
    (
        SELECT COALESCE(jsonb_agg(DISTINCT field ORDER BY field), '[]'::jsonb)
        FROM (SELECT unnest(mismatch_fields) AS field FROM verdict) names
    ) AS mismatch_fields,
    COALESCE(
        (SELECT jsonb_agg(diagnostics.entry ORDER BY diagnostics.ordinal)
         FROM item_diagnostics diagnostics),
        '[]'::jsonb
    ) AS item_diagnostics