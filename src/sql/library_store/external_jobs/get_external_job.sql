SELECT id,
       remote_task_id,
       status,
       remote_status,
       next_poll_at,
       deadline_at,
       error_message
FROM context69.task_external_jobs
WHERE item_id = $1
  AND provider = $2
