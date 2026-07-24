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
WHERE id = $1
