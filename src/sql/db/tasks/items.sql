SELECT item.id,
       item.task_id,
       item.ordinal,
       item.status,
       item.resource_id,
       item.file_id,
       item.stage,
       item.waiting_reason,
       item.dependency_key,
       item.next_attempt_at,
       item.failure_stage,
       item.error_message,
       item.attempt_count,
       item.retryable,
       item.created_at,
       item.started_at,
       item.finished_at,
       job.provider AS external_job_provider,
       job.remote_task_id AS external_job_remote_task_id,
       job.status AS external_job_status,
       job.remote_status AS external_job_remote_status,
       job.submitted_at AS external_job_submitted_at,
       job.last_polled_at AS external_job_last_polled_at,
       job.next_poll_at AS external_job_next_poll_at,
       job.deadline_at AS external_job_deadline_at,
       job.error_message AS external_job_error_message
FROM context69.task_items item
LEFT JOIN context69.task_external_jobs job
       ON job.item_id = item.id
      AND job.provider = 'docling'
WHERE item.task_id = $1
ORDER BY item.ordinal
LIMIT $2 OFFSET $3
