SELECT id,
       remote_task_id,
       status,
       remote_status,
       submitted_at,
       next_poll_at,
       deadline_at,
       error_message,
       submission_count
FROM context69.task_external_jobs
WHERE item_id = $1
  AND provider = $2
ORDER BY submitted_at DESC, created_at DESC
LIMIT 1
