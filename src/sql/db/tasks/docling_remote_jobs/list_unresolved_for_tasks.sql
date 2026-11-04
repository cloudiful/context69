-- List the fail-closed Docling submissions of a bounded task set (issue 1172 P5A).
--
-- The retry, resume and rerun entrypoints consult this list before reopening
-- work: a durable intent or an unknown outcome has no acknowledgement yet, so
-- those actions must refuse instead of starting a second submission.
--
-- `$1` is the task-id array. Ordering keeps repeated refusals deterministic.
SELECT
    id,
    task_id,
    item_id,
    status,
    dispatch_attempted
FROM context69.task_docling_remote_jobs
WHERE task_id = ANY($1::uuid[])
  AND status IN ('submitting', 'submission_uncertain')
ORDER BY task_id, created_at, id
