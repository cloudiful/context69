-- Durable owned Docling remote cleanup outbox (issue #84 P2).
--
-- Local cancellation terminalizes immediately (204) but must not abandon the
-- fork remote: the same transaction records an immutable cleanup request with
-- the submit-time endpoint snapshot. A background worker claims due rows with
-- FOR UPDATE SKIP LOCKED, cancels queued remotes (POST, no body, 5s) and
-- observes started remotes to terminal via immediate status (?wait=0, 5s),
-- never fetching results and never occupying a local processing slot.
--
-- No foreign-key cascade: cleanup rows must survive task/item purge so a
-- deleted task's remote is still terminated (or visibly terminal/deadline).
-- Identity is (endpoint, remote_task_id) for tracked submits; legacy rows
-- without an endpoint are idempotent by job id and never issue HTTP.

ALTER TABLE context69.task_docling_remote_jobs
    ADD COLUMN IF NOT EXISTS endpoint_base_url TEXT;

CREATE TABLE IF NOT EXISTS context69.task_docling_remote_cleanup (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    remote_job_id UUID NOT NULL,
    task_id UUID NOT NULL,
    item_id UUID NOT NULL,
    remote_task_id TEXT NOT NULL,
    endpoint_base_url TEXT,
    reason TEXT NOT NULL DEFAULT 'business_cancel',
    status TEXT NOT NULL DEFAULT 'pending',
    outcome TEXT,
    attempt_count INTEGER NOT NULL DEFAULT 0,
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at TIMESTAMPTZ,
    cleanup_deadline_at TIMESTAMPTZ NOT NULL DEFAULT now() + interval '24 hours',
    claim_token UUID,
    claim_until TIMESTAMPTZ,
    last_error TEXT,
    CONSTRAINT chk_task_docling_remote_cleanup_status
        CHECK (status IN ('pending', 'cancelling', 'awaiting_terminal', 'done', 'unsupported', 'failed')),
    CONSTRAINT chk_task_docling_remote_cleanup_reason
        CHECK (reason IN ('business_cancel', 'deadline', 'permanent_failure', 'orphan', 'purge'))
);

-- Tracked submits are unique by immutable identity; legacy NULL-endpoint rows
-- fall back to job-id idempotency so repeated enqueue never duplicates.
CREATE UNIQUE INDEX IF NOT EXISTS uq_docling_cleanup_endpoint_remote
    ON context69.task_docling_remote_cleanup (endpoint_base_url, remote_task_id)
    WHERE endpoint_base_url IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS uq_docling_cleanup_legacy_job
    ON context69.task_docling_remote_cleanup (remote_job_id)
    WHERE endpoint_base_url IS NULL;

CREATE INDEX IF NOT EXISTS idx_docling_cleanup_due
    ON context69.task_docling_remote_cleanup (status, next_attempt_at)
    WHERE status IN ('pending', 'cancelling', 'awaiting_terminal');

CREATE INDEX IF NOT EXISTS idx_docling_cleanup_task
    ON context69.task_docling_remote_cleanup (task_id);

CREATE INDEX IF NOT EXISTS idx_docling_cleanup_item
    ON context69.task_docling_remote_cleanup (item_id);
