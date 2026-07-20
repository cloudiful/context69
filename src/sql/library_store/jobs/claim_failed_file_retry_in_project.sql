WITH claimed AS (
    UPDATE context69.library_files
    SET ingest_status = 'pending',
        error_message = NULL,
        ingested_at = NULL,
        updated_at = now()
    WHERE group_id = $1
      AND id = $2
      AND ingest_status = 'failed'
    RETURNING id, group_id, visibility
)
INSERT INTO context69.library_ingest_jobs (
    id,
    group_id,
    visibility,
    file_id,
    status
)
SELECT $3, group_id, visibility, id, 'pending'
FROM claimed
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
