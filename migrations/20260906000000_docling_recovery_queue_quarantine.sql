-- Queue-only Docling recovery and stale `submitting` quarantine (issue #118 phase 4).
--
-- 1. Adds the `orphaned` external-job status: an explicit, non-active marker
--    for stale `submitting` rows whose remote submission outcome is uncertain.
--    `orphaned` is deliberately NOT in the active set
--    (`submitting`/`pending`/`running`), so quarantined rows stop blocking
--    terminal-task cleanup/purge and stop counting toward the Docling remote
--    admission ceiling. Quarantine never claims the remote job was cancelled:
--    the original `remote_status`/`error_message` are preserved (the reason is
--    appended, never overwritten) and the quarantine reason, actor, and
--    timestamp are recorded in new columns plus one audit row per job.
--    Only the explicit admin quarantine API moves rows to `orphaned`; no
--    background job performs this transition automatically.
--
-- 2. This migration is schema-only and touches no data rows. Existing
--    `submitting`/`pending`/`running` rows keep their status, so the status
--    check change is purely additive and compatible with all current data.

ALTER TABLE context69.task_external_jobs
    DROP CONSTRAINT IF EXISTS chk_task_external_jobs_status;

ALTER TABLE context69.task_external_jobs
    ADD CONSTRAINT chk_task_external_jobs_status
    CHECK (status IN ('submitting', 'pending', 'running', 'success', 'failure', 'timed_out', 'cancelled', 'orphaned'));

ALTER TABLE context69.task_external_jobs
    ADD COLUMN IF NOT EXISTS quarantine_reason TEXT,
    ADD COLUMN IF NOT EXISTS quarantined_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS quarantined_by TEXT;

CREATE TABLE IF NOT EXISTS context69.task_external_job_quarantine_audit (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    task_id uuid NOT NULL REFERENCES context69.tasks(id) ON DELETE CASCADE,
    item_id uuid NOT NULL REFERENCES context69.task_items(id) ON DELETE CASCADE,
    external_job_id uuid NOT NULL REFERENCES context69.task_external_jobs(id) ON DELETE CASCADE,
    provider text NOT NULL,
    old_remote_task_id text,
    old_remote_status text,
    old_error_message text,
    actor_user_id BIGINT NOT NULL REFERENCES context69.users(id) ON DELETE RESTRICT,
    actor_login_name TEXT NOT NULL,
    reason TEXT NOT NULL,
    quarantined_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_task_external_job_quarantine_audit_item
    ON context69.task_external_job_quarantine_audit (item_id, quarantined_at DESC);

CREATE INDEX IF NOT EXISTS idx_task_external_jobs_status_submitting
    ON context69.task_external_jobs (provider, status, submitted_at)
    WHERE status = 'submitting';

CREATE INDEX IF NOT EXISTS idx_task_external_jobs_status_orphaned
    ON context69.task_external_jobs (provider, status)
    WHERE status = 'orphaned';
