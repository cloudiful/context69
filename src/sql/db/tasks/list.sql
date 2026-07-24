SELECT
    id,
    user_id,
    group_id,
    kind,
    status,
    group_path,
    source_key,
    total_count,
    queued_count,
    running_count,
    succeeded_count,
    failed_count,
    cancelled_count,
    failure_stage,
    error_summary,
    created_at,
    started_at,
    finished_at,
    updated_at
FROM context69.tasks
WHERE user_id = $1
  AND ($2::text IS NULL OR kind = $2)
  AND ($3::text IS NULL OR status = $3)
ORDER BY created_at DESC
LIMIT $4 OFFSET $5
