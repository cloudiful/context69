-- Keep each external submission as a separate row.
--
-- A single (item_id, provider) row cannot preserve the remote task id that
-- was superseded during recovery. The latest-job queries select the newest
-- submission explicitly after this index replaces the old uniqueness rule.
DROP INDEX IF EXISTS context69.uq_task_external_jobs_item_provider;

CREATE INDEX IF NOT EXISTS idx_task_external_jobs_item_provider_submitted
    ON context69.task_external_jobs (item_id, provider, submitted_at DESC, created_at DESC);

ALTER TABLE context69.task_docling_recovery_audit
    ADD COLUMN IF NOT EXISTS old_submission_count INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS new_submission_count INTEGER NOT NULL DEFAULT 0;

UPDATE context69.task_items
SET waiting_since = COALESCE(updated_at, created_at)
WHERE status = 'waiting'
  AND waiting_since IS NULL;

ALTER TABLE context69.task_external_jobs
    DROP CONSTRAINT IF EXISTS chk_task_external_jobs_status;

ALTER TABLE context69.task_external_jobs
    ADD CONSTRAINT chk_task_external_jobs_status
    CHECK (status IN ('submitting', 'pending', 'running', 'success', 'failure', 'timed_out', 'cancelled'));
