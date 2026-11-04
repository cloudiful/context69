-- Retained Docling submission history of one task (issue 1172 P5B1).
--
-- A recovery archives the accepted unknown row as `superseded` and a later
-- dispatch appends a distinct row, so the regressions assert the old diagnostic
-- survives beside the current intent. Each row carries its item's state, so a
-- recovery can also prove it never reopened the old parked item. $1 is the task.
SELECT
    job.id,
    job.item_id,
    job.status AS job_status,
    job.dispatch_attempted,
    job.submission_identity,
    job.last_error,
    job.created_at,
    job.finished_at,
    item.status AS item_status,
    item.waiting_reason AS item_waiting_reason
FROM context69.task_docling_remote_jobs AS job
JOIN context69.task_items AS item ON item.id = job.item_id
WHERE job.task_id = $1
ORDER BY job.created_at, job.id
