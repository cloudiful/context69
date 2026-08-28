-- Phase 6: extraction retry observability
-- Adds failure classification persistence and delayed retry scheduling.

ALTER TABLE context69.document_extraction_jobs
    ADD COLUMN IF NOT EXISTS failure_class TEXT,
    ADD COLUMN IF NOT EXISTS next_attempt_at TIMESTAMPTZ;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'chk_document_extraction_job_failure_class'
    ) THEN
        ALTER TABLE context69.document_extraction_jobs
            ADD CONSTRAINT chk_document_extraction_job_failure_class
            CHECK (failure_class IS NULL OR failure_class IN ('transient', 'quota_exceeded', 'permanent'));
    END IF;
END $$;

-- Supports efficient scan for due queued retries (next_attempt_at IS NULL or <= now()).
CREATE INDEX IF NOT EXISTS idx_document_extraction_jobs_queued_retry_due
    ON context69.document_extraction_jobs (next_attempt_at, created_at, id)
    WHERE status = 'queued';
