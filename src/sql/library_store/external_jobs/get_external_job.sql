SELECT id,
       item_id,
       provider,
       remote_task_id,
       status,
       remote_status,
       submitted_at,
       last_polled_at,
       next_poll_at,
       deadline_at,
       error_message,
       updated_at
FROM context69.task_external_jobs
WHERE item_id = $1
  AND provider = $2
