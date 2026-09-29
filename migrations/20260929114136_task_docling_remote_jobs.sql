-- Durable Docling remote-job state (issue #639 phase docling-remote-job-store).
--
-- Fresh persistence groundwork for the unified Docling polling lifecycle.
-- `task_external_jobs` was dropped by 20260920180515_drop_docling_async.sql,
-- so there is no data to migrate; this table is the new source of truth for
-- resumability across restarts and replicas.
--
-- One TaskItem owns at most one active remote job: the partial unique index
-- `uq_task_docling_remote_jobs_item_active` enforces a single
-- ('pending','running') row per item_id while terminal history
-- ('succeeded','failed','cancelled','timed_out') remains queryable.
-- `remote_task_id` is globally unique so duplicate deliveries and double
-- submits are rejected at the database boundary.
--
-- The remote lease (`lease_token`/`lease_until`) is independent from the
-- task-item lease and fences the poll sweep: only the holder of the current
-- lease_token may record a poll outcome or finish the job, and the sweep
-- claims due rows with `FOR UPDATE SKIP LOCKED`. `next_poll_at` is the due
-- time (client-side minimum interval/backoff even when Docling answers
-- immediately), `deadline_at` bounds the whole conversion, and
-- `remote_status`/`last_error`/`attempt_count` carry the last observed
-- Docling state for observability and retry decisions.

CREATE TABLE IF NOT EXISTS context69.task_docling_remote_jobs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    task_id UUID NOT NULL REFERENCES context69.tasks(id) ON DELETE CASCADE,
    item_id UUID NOT NULL REFERENCES context69.task_items(id) ON DELETE CASCADE,
    provider TEXT NOT NULL DEFAULT 'docling',
    remote_task_id TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    remote_status TEXT,
    attempt_count INTEGER NOT NULL DEFAULT 0,
    next_poll_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_polled_at TIMESTAMPTZ,
    deadline_at TIMESTAMPTZ,
    lease_token UUID,
    lease_until TIMESTAMPTZ,
    last_error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at TIMESTAMPTZ,
    CONSTRAINT chk_task_docling_remote_jobs_status
        CHECK (status IN ('pending', 'running', 'succeeded', 'failed', 'cancelled', 'timed_out'))
);

CREATE UNIQUE INDEX IF NOT EXISTS uq_task_docling_remote_jobs_remote_task_id
    ON context69.task_docling_remote_jobs (remote_task_id);

CREATE UNIQUE INDEX IF NOT EXISTS uq_task_docling_remote_jobs_item_active
    ON context69.task_docling_remote_jobs (item_id)
    WHERE status IN ('pending', 'running');

CREATE INDEX IF NOT EXISTS idx_task_docling_remote_jobs_poll_due
    ON context69.task_docling_remote_jobs (status, next_poll_at)
    WHERE status IN ('pending', 'running');

CREATE INDEX IF NOT EXISTS idx_task_docling_remote_jobs_deadline
    ON context69.task_docling_remote_jobs (deadline_at)
    WHERE status IN ('pending', 'running') AND deadline_at IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_task_docling_remote_jobs_task
    ON context69.task_docling_remote_jobs (task_id);
