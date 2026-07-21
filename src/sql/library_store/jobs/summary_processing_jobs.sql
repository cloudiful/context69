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
        group_row.id AS group_id,
        job.id AS job_id,
        'url_import'::TEXT AS kind,
        job.dedupe_key,
        job.file_id,
        CASE job.status
            WHEN 'queued' THEN 'pending'
            WHEN 'downloading' THEN 'running'
            WHEN 'ingesting' THEN 'running'
            ELSE job.status
        END AS status,
        group_row.id IN (SELECT group_id FROM managed_groups) AS can_retry,
        job.created_at,
        job.updated_at
    FROM context69.library_url_import_jobs job
    JOIN context69.groups group_row ON group_row.id = job.group_id
    WHERE (group_row.visibility = 'public' OR group_row.id = ANY($2::BIGINT[]))

    UNION ALL

    SELECT
        group_row.id AS group_id,
        job.id AS job_id,
        'ingest'::TEXT AS kind,
        NULL::TEXT AS dedupe_key,
        job.file_id,
        job.status,
        group_row.id IN (SELECT group_id FROM managed_groups) AS can_retry,
        job.created_at,
        job.updated_at
    FROM context69.library_ingest_jobs job
    JOIN context69.groups group_row ON group_row.id = job.group_id
    WHERE (group_row.visibility = 'public' OR group_row.id = ANY($2::BIGINT[]))
      AND NOT EXISTS (
          SELECT 1
          FROM context69.library_url_import_jobs url_job
          WHERE url_job.ingest_job_id = job.id
      )
), retry_candidates AS (
    SELECT job_id
    FROM (
        SELECT
            job_id,
            status,
            ROW_NUMBER() OVER (
                PARTITION BY file_id
                ORDER BY created_at DESC, job_id DESC
            ) AS row_number
        FROM jobs
        WHERE kind = 'ingest'
          AND can_retry
          AND file_id IS NOT NULL
    ) latest_ingest
    WHERE row_number = 1
      AND status = 'failed'

    UNION ALL

    SELECT job_id
    FROM (
        SELECT
            job_id,
            status,
            ROW_NUMBER() OVER (
                PARTITION BY group_id, dedupe_key
                ORDER BY created_at DESC, job_id DESC
            ) AS row_number
        FROM jobs
        WHERE kind = 'url_import'
          AND can_retry
    ) latest_url_import
    WHERE row_number = 1
      AND status = 'failed'
)
SELECT
    COUNT(*) FILTER (WHERE status = 'pending') AS "pending_count!: i64",
    COUNT(*) FILTER (WHERE status = 'running') AS "running_count!: i64",
    COUNT(*) FILTER (WHERE status = 'failed') AS "failed_count!: i64",
    COUNT(*) FILTER (
        WHERE status IN ('pending', 'running') AND updated_at < $3
    ) AS "stuck_count!: i64",
    (
        SELECT COUNT(*)
        FROM retry_candidates
    ) AS "retryable_failed_count!: i64",
    COUNT(*) FILTER (
        WHERE can_retry
          AND status IN ('pending', 'running')
          AND updated_at < $3
    ) AS "cleanupable_stuck_count!: i64"
FROM jobs
