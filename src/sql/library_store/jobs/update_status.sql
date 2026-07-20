UPDATE context69.library_ingest_jobs
SET status = $2,
    docling_task_id = COALESCE($3, docling_task_id),
    failure_stage = $4,
    error_message = $5,
    started_at = COALESCE($6, started_at),
    finished_at = $7,
    updated_at = now()
WHERE id = $1
RETURNING
    group_id,
    (SELECT group_key FROM context69.groups WHERE id = group_id) AS "group_key!",
    (SELECT full_path FROM context69.groups WHERE id = group_id) AS "group_path!",
    visibility,
    id,
    file_id,
    status,
    docling_task_id,
    failure_stage,
    error_message,
    created_at,
    started_at,
    finished_at,
    updated_at
