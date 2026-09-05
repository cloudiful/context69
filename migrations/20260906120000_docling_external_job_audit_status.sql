-- Record the original external-job status in recovery and quarantine audits (issue #129 phase 1).
--
-- `task_docling_recovery_audit` pins `old_remote_task_id`/`old_remote_status`
-- but never recorded the superseded row's own `status` (`pending`/`running`/
-- `submitting`/terminal), so operators could not tell which state was
-- superseded without re-reading `task_external_jobs` history. Likewise
-- `task_external_job_quarantine_audit` pins the old remote id/status/error
-- but not the pre-quarantine `status` (always `submitting` under the current
-- eligibility guard, recorded explicitly for audit completeness and to keep
-- the transition self-describing if eligibility ever widens).
--
-- This migration adds a nullable `old_status TEXT` to both audit tables. It
-- is purely additive and backward compatible:
--   * existing audit rows keep NULL (unknown) rather than a backfilled guess;
--   * no rows are updated, no defaults are imposed, no indexes are added;
--   * no `task_external_jobs`, task, item, attempt, or idempotency state is
--     touched and no production quarantine/retry is triggered.
-- The field name `old_status` matches `task_external_jobs.status` and the
-- existing `old_remote_status`/`old_remote_task_id`/`old_error_message`
-- audit prefix convention.

ALTER TABLE context69.task_docling_recovery_audit
    ADD COLUMN IF NOT EXISTS old_status TEXT;

ALTER TABLE context69.task_external_job_quarantine_audit
    ADD COLUMN IF NOT EXISTS old_status TEXT;
