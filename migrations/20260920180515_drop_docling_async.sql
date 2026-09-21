-- Drop the Docling async submit/poll storage (issue #529 Task 2).
--
-- The blocking worker converts documents inline through
-- `prepare_file_sections_for_task` (holding the Docling permit for the whole
-- conversion), so no code writes remote job rows, quarantine audit rows, or
-- recovery audit rows any more. Three tables are therefore dropped:
--   * context69.task_external_jobs held the remote task id, poll cadence,
--     deadline, and submission history for the removed poll path and the
--     removed persistent remote-admission gate.
--   * context69.task_external_job_quarantine_audit was the orphaned
--     `submitting` quarantine trail for those jobs; it must be dropped before
--     `task_external_jobs` because it carries the foreign key.
--   * context69.task_docling_recovery_audit held the operator audit trail of
--     the removed immediate/queue-only recovery chains.
--
-- Destructive and irreversible: the audit history is not recoverable after
-- this migration. No task, task_item, or task_attempt row is touched here.

DROP TABLE IF EXISTS context69.task_external_job_quarantine_audit;
DROP TABLE IF EXISTS context69.task_docling_recovery_audit;
DROP TABLE IF EXISTS context69.task_external_jobs;

-- The NOTIFY trigger on context69.task_external_jobs disappears with the
-- table; drop its now-unreachable trigger function too.
DROP FUNCTION IF EXISTS context69.notify_task_external_job_event();
