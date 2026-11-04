-- Archive the validated unknown Docling submissions of one task (issue 1172 P5B1).
--
-- Only the exact `submission_uncertain` rows already validated and locked by
-- the caller are archived. Provenance, frozen identity, dispatch flag,
-- `created_at`, `last_error`, remote status and the remote id stay untouched:
-- the row becomes `superseded` with a finish timestamp so the history and the
-- original diagnostic survive. `$1` task, `$2` validated job ids.
UPDATE context69.task_docling_remote_jobs AS job
SET status = 'superseded',
    remote_status = COALESCE(job.remote_status, job.status),
    finished_at = now(),
    updated_at = now()
WHERE job.task_id = $1
  AND job.id = ANY($2::uuid[])
  AND job.status = 'submission_uncertain'
RETURNING job.item_id
