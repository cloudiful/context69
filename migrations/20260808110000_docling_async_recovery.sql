-- Recovery for the docling whole-document async conversion switch.
--
-- 1. Docling conversion timeouts no longer trip the global dependency gate, so
--    reset the gate that was repeatedly opened by a single slow PDF.
-- 2. Items parked on that gate are requeued; the new binary submits each file
--    as one whole-document async task with a one-hour deadline.
-- 3. The old ten-minute task deadline is too short for whole documents, so the
--    stored setting is bumped to one hour.

UPDATE context69.library_dependency_gates
SET state = 'closed',
    failure_count = 0,
    next_probe_at = NULL,
    probe_lease_token = NULL,
    probe_lease_expires_at = NULL,
    last_error = NULL,
    updated_at = now()
WHERE dependency_key = 'docling'
  AND state <> 'closed';

UPDATE context69.task_items
SET status = 'queued',
    waiting_reason = NULL,
    dependency_key = NULL,
    next_attempt_at = NULL,
    error_message = NULL,
    updated_at = now()
WHERE status = 'waiting'
  AND waiting_reason = 'dependency'
  AND dependency_key = 'docling';

UPDATE context69.docling_settings
SET task_timeout_secs = 3600,
    updated_at = now()
WHERE task_timeout_secs IS NOT NULL
  AND task_timeout_secs <= 600;

-- The five-page-per-task split is gone: whole documents are submitted as one
-- async Docling task, so the per-task page budget setting is dead.
ALTER TABLE context69.runtime_file_library_settings
    DROP COLUMN IF EXISTS pdf_pages_per_task;
