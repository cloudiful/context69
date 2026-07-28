WITH finished AS (
UPDATE context69.library_ingest_jobs
SET status = $3,
    failure_stage = $4,
    error_message = $5,
    lease_token = NULL,
    lease_expires_at = NULL,
    finished_at = now(),
    updated_at = now()
WHERE id = $1
  AND lease_token = $2
  AND status = 'running'
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
), file_updated AS (
    UPDATE context69.library_files file
    SET ingest_status = finished.status,
        error_message = finished.error_message,
        ingested_at = CASE
            WHEN finished.status = 'succeeded' THEN now()
            ELSE NULL
        END,
        updated_at = now()
    FROM finished
    WHERE file.id = finished.file_id
    RETURNING file.id
), url_updated AS (
    UPDATE context69.library_url_import_jobs url_job
    SET status = CASE finished.status
            WHEN 'succeeded' THEN 'succeeded'
            WHEN 'failed' THEN 'failed'
        END,
        error_code = CASE
            WHEN finished.status = 'failed'
                THEN COALESCE('ingest_' || finished.failure_stage, 'ingest_failed')
            ELSE NULL
        END,
        error_message = CASE
            WHEN finished.status = 'failed' THEN finished.error_message
            ELSE NULL
        END,
        failure_stage = CASE
            WHEN finished.status = 'failed' THEN finished.failure_stage
            ELSE NULL
        END,
        next_attempt_at = NULL,
        finished_at = COALESCE(finished.finished_at, now()),
        lease_token = NULL,
        lease_expires_at = NULL,
        updated_at = now()
    FROM finished
    JOIN file_updated ON file_updated.id = finished.file_id
    WHERE url_job.ingest_job_id = finished.id
      AND url_job.status IN ('ingesting', 'queued')
)
SELECT
    finished.group_id AS "group_id!",
    finished.group_key AS "group_key!",
    finished.group_path AS "group_path!",
    finished.visibility AS "visibility!",
    finished.id AS "id!",
    finished.file_id AS "file_id!",
    finished.status AS "status!",
    finished.docling_task_id,
    finished.failure_stage,
    finished.error_message,
    finished.created_at AS "created_at!",
    finished.started_at,
    finished.finished_at,
    finished.updated_at AS "updated_at!"
FROM finished
