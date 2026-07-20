SELECT
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
FROM context69.library_ingest_jobs
WHERE group_id = $1
  AND id = $2
