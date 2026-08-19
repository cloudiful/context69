-- Include uncertain local submissions in the active-job deadline index.
-- These rows have no trustworthy remote id yet and must remain visible to
-- operational health checks until an administrator resolves them.
DROP INDEX IF EXISTS context69.idx_task_external_jobs_deadline;

CREATE INDEX idx_task_external_jobs_deadline
    ON context69.task_external_jobs (status, deadline_at)
    WHERE status IN ('submitting', 'pending', 'running');
