ALTER TABLE context69.task_docling_remote_jobs
    ALTER COLUMN remote_task_id DROP NOT NULL,
    ADD COLUMN dispatch_attempted BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN submission_execution_task_id UUID,
    ADD COLUMN submission_generation BIGINT,
    ADD COLUMN submission_identity JSONB NOT NULL DEFAULT '{}'::jsonb,
    DROP CONSTRAINT chk_task_docling_remote_jobs_status,
    ADD CONSTRAINT chk_task_docling_remote_jobs_status
        CHECK (status IN (
            'submitting', 'submission_uncertain', 'superseded',
            'pending', 'running', 'succeeded', 'failed', 'cancelled', 'timed_out'
        )),
    ADD CONSTRAINT chk_task_docling_remote_jobs_known_id
        CHECK (status NOT IN ('pending', 'running') OR remote_task_id IS NOT NULL),
    ADD CONSTRAINT chk_task_docling_remote_jobs_submission_identity
        CHECK (jsonb_typeof(submission_identity) = 'object');

DROP INDEX context69.uq_task_docling_remote_jobs_item_active;

CREATE UNIQUE INDEX uq_task_docling_remote_jobs_item_unresolved
    ON context69.task_docling_remote_jobs (item_id)
    WHERE status IN ('submitting', 'submission_uncertain', 'pending', 'running');
