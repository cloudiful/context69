-- Schema changes for Docling waiting-item recovery.
--
-- 1. `task_items.waiting_since` records when an item most recently entered the
--    `waiting` state. It lets background health checks alert on items that
--    have been parked for too long (e.g. >30 minutes) and gives operators a
--    timestamp to reason about during recovery. The column is set on every
--    transition into `waiting` (handled by `wait_item.sql` and
--    `dependency_wait`), and cleared when the item leaves `waiting` for a
--    productive state.
--
-- 2. `task_external_jobs.submission_count` records how many times the same
--    item has been resubmitted to an external provider. A retry that simply
--    bumps this counter without first invalidating the old remote task id is
--    the difference between "tried again with the same id" (bug) and
--    "re-submitted a fresh task" (correct). The recovery audit writes the
--    pre-recovery remote id and the new submission count side by side.
--
-- 3. `task_docling_recovery_audit` records every recovery invocation. The
--    audit row pins the old remote task id, the operator, the reason, and
--    the recovery timestamp so production canary recoveries can be
--    reconstructed. Rows are append-only.
--
-- 4. New indexes accelerate health queries:
--    - `idx_task_items_waiting_since` finds the oldest waiting item for the
--      waiting-age alert and the `processing_health` snapshot.
--    - `idx_task_items_waiting_dependency` finds items parked on a
--      dependency (notably `docling`) for dependency-age alerts.
--    - `idx_task_external_jobs_deadline` finds active jobs whose deadline
--      has passed and need a forced terminal status.
--    - `idx_task_external_jobs_provider_status` powers the
--      `processing_health` view of stuck remote jobs.
--
-- This migration is intentionally schema-only. It MUST NOT mutate or fail any
-- in-flight task: production recovery is performed by the new admin API
-- (`POST /v1/admin/tasks/{task_id}/recover`) or by an explicit one-shot ops
-- command, never by a DDL step.

ALTER TABLE context69.task_items
    ADD COLUMN IF NOT EXISTS waiting_since TIMESTAMPTZ;

ALTER TABLE context69.task_external_jobs
    ADD COLUMN IF NOT EXISTS submission_count INTEGER NOT NULL DEFAULT 1
        CHECK (submission_count >= 1);

CREATE TABLE IF NOT EXISTS context69.task_docling_recovery_audit (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    task_id uuid NOT NULL REFERENCES context69.tasks(id) ON DELETE CASCADE,
    item_id uuid NOT NULL REFERENCES context69.task_items(id) ON DELETE CASCADE,
    actor_user_id BIGINT NOT NULL REFERENCES context69.users(id) ON DELETE RESTRICT,
    actor_login_name TEXT NOT NULL,
    reason TEXT NOT NULL,
    old_external_job_id uuid REFERENCES context69.task_external_jobs(id) ON DELETE SET NULL,
    old_remote_task_id TEXT,
    old_remote_status TEXT,
    new_external_job_id uuid REFERENCES context69.task_external_jobs(id) ON DELETE SET NULL,
    new_remote_task_id TEXT,
    recovered_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_task_docling_recovery_audit_task
    ON context69.task_docling_recovery_audit (task_id, recovered_at DESC);

CREATE INDEX IF NOT EXISTS idx_task_items_waiting_since
    ON context69.task_items (waiting_since)
    WHERE status = 'waiting';

CREATE INDEX IF NOT EXISTS idx_task_items_waiting_dependency
    ON context69.task_items (dependency_key, waiting_since)
    WHERE status = 'waiting' AND waiting_reason = 'dependency';

CREATE INDEX IF NOT EXISTS idx_task_external_jobs_deadline
    ON context69.task_external_jobs (status, deadline_at)
    WHERE status IN ('pending', 'running');

CREATE INDEX IF NOT EXISTS idx_task_external_jobs_provider_status
    ON context69.task_external_jobs (provider, status, last_polled_at);
