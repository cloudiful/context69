-- Controlled remediation for stale Docling external jobs and parent task aggregates.
--
-- Operator confirmation: backup and maintenance window are ready; db-mcp is
-- read-only so this repair is delivered as an idempotent, transactional
-- migration to run during the next deployment window.
--
-- 1. Local-only reconciliation of Docling external jobs attached to terminal items.
--    - Only provider='docling' rows with status IN ('pending','running') are touched.
--    - Parent task_item must be terminal (succeeded/failed/cancelled) in this snapshot.
--    - 'submitting' rows are never touched: remote submission outcome is uncertain
--      and must remain manual-recovery-required.
--    - Active items (queued/running/waiting) are left alone.
--    - No remote HTTP cancellation is performed. remote_status is preserved via
--      COALESCE, and error_message explicitly states remote cancellation was not
--      requested. Historical rows are reconciled (no LIMIT to latest).
--    - Predicate is idempotent: re-running the UPDATE touches 0 rows once every
--      pending/running terminal job is already cancelled.
--
-- 2. Recompute denormalized parent task fields for complete terminal tasks where
--    stored aggregates differ from task_items.
--    - Eligible only when the task's item set is complete (no active item:
--      queued=running=waiting=0) and terminal sum equals total_count, so
--      incomplete or active tasks are never altered.
--    - And only where any of the six stored counters is distinct from the actual
--      aggregate (IS DISTINCT FROM guard) so already-consistent tasks are a no-op.
--    - Sets all six counters, failure_stage/error_summary from the first failed
--      item ordered by ordinal where applicable, clears active stage/
--      waiting_reason/dependency_key/next_attempt_at, clears leases when terminal,
--      sets canonical terminal status and finished_at, preserving recompute.sql
--      semantics. The UPDATE is idempotent via the counter guard.
--
-- Both updates run in the migration's implicit transaction and are therefore atomic.
-- SQLx bookkeeping prevents re-execution, but the statements themselves are safe to
-- re-apply.

-- 1. Locally cancel pending/running Docling jobs whose item is already terminal.
UPDATE context69.task_external_jobs AS job
SET status = 'cancelled',
    remote_status = COALESCE(job.remote_status, job.status),
    error_message = COALESCE(
        job.error_message,
        'task item is terminal; local external job cancelled without remote cancellation'
    ),
    last_polled_at = now(),
    updated_at = now()
WHERE job.provider = 'docling'
  AND job.status IN ('pending', 'running')
  AND EXISTS (
      SELECT 1
      FROM context69.task_items ti
      WHERE ti.id = job.item_id
        AND ti.status IN ('succeeded', 'failed', 'cancelled')
  );

-- 2. Recompute parent task aggregates only for complete terminal tasks whose counters mismatch.
WITH counts AS (
    SELECT
        task_id,
        count(*) FILTER (WHERE status = 'queued')::bigint AS queued_count,
        count(*) FILTER (WHERE status = 'running')::bigint AS running_count,
        count(*) FILTER (WHERE status = 'waiting')::bigint AS waiting_count,
        count(*) FILTER (WHERE status = 'succeeded')::bigint AS succeeded_count,
        count(*) FILTER (WHERE status = 'failed')::bigint AS failed_count,
        count(*) FILTER (WHERE status = 'cancelled')::bigint AS cancelled_count
    FROM context69.task_items
    GROUP BY task_id
)
UPDATE context69.tasks t
SET queued_count = counts.queued_count,
    running_count = counts.running_count,
    waiting_count = counts.waiting_count,
    succeeded_count = counts.succeeded_count,
    failed_count = counts.failed_count,
    cancelled_count = counts.cancelled_count,
    failure_stage = (
        SELECT failure_stage
        FROM context69.task_items
        WHERE task_id = t.id AND status = 'failed' AND failure_stage IS NOT NULL
        ORDER BY ordinal
        LIMIT 1
    ),
    error_summary = (
        SELECT error_message
        FROM context69.task_items
        WHERE task_id = t.id AND status = 'failed' AND error_message IS NOT NULL
        ORDER BY ordinal
        LIMIT 1
    ),
    stage = NULL,
    waiting_reason = NULL,
    dependency_key = NULL,
    next_attempt_at = NULL,
    lease_token = NULL,
    lease_until = NULL,
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
        THEN COALESCE(t.finished_at, now())
        ELSE NULL
    END,
    updated_at = now()
FROM counts
WHERE t.id = counts.task_id
  AND counts.queued_count = 0
  AND counts.running_count = 0
  AND counts.waiting_count = 0
  AND counts.succeeded_count + counts.failed_count + counts.cancelled_count = t.total_count
  AND (
      t.queued_count IS DISTINCT FROM counts.queued_count
      OR t.running_count IS DISTINCT FROM counts.running_count
      OR t.waiting_count IS DISTINCT FROM counts.waiting_count
      OR t.succeeded_count IS DISTINCT FROM counts.succeeded_count
      OR t.failed_count IS DISTINCT FROM counts.failed_count
      OR t.cancelled_count IS DISTINCT FROM counts.cancelled_count
  );
