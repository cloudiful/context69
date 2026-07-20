INSERT INTO context69.library_ingest_jobs (
    id,
    group_id,
    visibility,
    file_id,
    status
)
SELECT
    $1,
    lf.group_id,
    lf.visibility,
    $2,
    'pending'
FROM context69.library_files lf
WHERE lf.id = $2
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
