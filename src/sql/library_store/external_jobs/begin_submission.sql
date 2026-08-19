WITH previous AS (
    SELECT COALESCE(MAX(submission_count), 0) AS submission_count
    FROM context69.task_external_jobs
    WHERE item_id = $1
      AND provider = $2
), inserted AS (
    INSERT INTO context69.task_external_jobs (
        item_id,
        provider,
        remote_task_id,
        status,
        submitted_at,
        next_poll_at,
        deadline_at,
        submission_count,
        updated_at
    )
    SELECT $1, $2, $3, 'submitting', now(), $4, $5,
           previous.submission_count + 1,
           now()
    FROM previous
    RETURNING id, submission_count
)
SELECT id, submission_count
FROM inserted
