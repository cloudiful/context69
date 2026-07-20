WITH RECURSIVE inherited_roles AS (
    SELECT gm.group_id, gm.role
    FROM context69.group_memberships gm
    WHERE gm.user_id = $1
    UNION ALL
    SELECT child.id, inherited_roles.role
    FROM context69.groups child
    JOIN inherited_roles ON child.parent_group_id = inherited_roles.group_id
), managed_groups AS (
    SELECT DISTINCT group_id
    FROM inherited_roles
    WHERE role IN ('owner', 'maintainer')
), jobs AS (
    SELECT
        'url_import'::TEXT AS kind,
        job.id AS job_id,
        group_row.group_key,
        group_row.full_path AS group_path,
        job.visibility,
        job.file_id,
        COALESCE(file.filename, job.requested_filename, job.source_url) AS filename,
        job.source_url,
        CASE job.status
            WHEN 'queued' THEN 'pending'
            WHEN 'downloading' THEN 'running'
            WHEN 'ingesting' THEN 'running'
            ELSE job.status
        END AS status,
        COALESCE(job.failure_stage, ingest.failure_stage) AS failure_stage,
        COALESCE(ingest.error_message, job.error_message) AS error_message,
        group_row.id IN (SELECT group_id FROM managed_groups) AS can_retry,
        job.created_at,
        COALESCE(job.started_at, ingest.started_at) AS started_at,
        COALESCE(job.finished_at, ingest.finished_at) AS finished_at,
        job.updated_at
    FROM context69.library_url_import_jobs job
    JOIN context69.groups group_row ON group_row.id = job.group_id
    LEFT JOIN context69.library_files file ON file.id = job.file_id
    LEFT JOIN context69.library_ingest_jobs ingest ON ingest.id = job.ingest_job_id
    WHERE (group_row.visibility = 'public' OR group_row.id = ANY($2::BIGINT[]))

    UNION ALL

    SELECT
        'ingest'::TEXT AS kind,
        job.id AS job_id,
        group_row.group_key,
        group_row.full_path AS group_path,
        job.visibility,
        job.file_id,
        file.filename,
        NULL::TEXT AS source_url,
        job.status,
        job.failure_stage,
        job.error_message,
        group_row.id IN (SELECT group_id FROM managed_groups) AS can_retry,
        job.created_at,
        job.started_at,
        job.finished_at,
        job.updated_at
    FROM context69.library_ingest_jobs job
    JOIN context69.groups group_row ON group_row.id = job.group_id
    JOIN context69.library_files file ON file.id = job.file_id
    WHERE (group_row.visibility = 'public' OR group_row.id = ANY($2::BIGINT[]))
      AND NOT EXISTS (
          SELECT 1
          FROM context69.library_url_import_jobs url_job
          WHERE url_job.ingest_job_id = job.id
      )
)
SELECT COUNT(*)
FROM jobs
WHERE ($3::TEXT IS NULL OR CONCAT_WS(' ', filename, group_key, group_path, source_url, error_message) ILIKE '%' || BTRIM($3::TEXT) || '%')
  AND ($4::TEXT IS NULL OR status = $4::TEXT)
  AND ($5::TEXT IS NULL OR failure_stage = $5::TEXT)
