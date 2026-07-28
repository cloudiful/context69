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
        job.status,
        job.failure_stage,
        job.error_message,
        job.created_at
    FROM context69.library_url_import_jobs job
    JOIN context69.groups group_row ON group_row.id = job.group_id
    WHERE (group_row.visibility = 'public' OR group_row.id = ANY($2::BIGINT[]))
      AND group_row.id IN (SELECT group_id FROM managed_groups)

    UNION ALL

    SELECT
        group_row.id AS group_id,
        job.id AS job_id,
        'ingest'::TEXT AS kind,
        NULL::TEXT AS dedupe_key,
        job.file_id,
        job.status,
        job.failure_stage,
        job.error_message,
        job.created_at
    FROM context69.library_ingest_jobs job
    JOIN context69.groups group_row ON group_row.id = job.group_id
    WHERE (group_row.visibility = 'public' OR group_row.id = ANY($2::BIGINT[]))
      AND group_row.id IN (SELECT group_id FROM managed_groups)
      AND NOT EXISTS (
          SELECT 1
          FROM context69.library_url_import_jobs url_job
          WHERE url_job.ingest_job_id = job.id
      )
), latest_ingest AS (
    SELECT group_id, job_id, kind, file_id, failure_stage, error_message, created_at
    FROM (
        SELECT
            jobs.*,
            ROW_NUMBER() OVER (
                PARTITION BY file_id
                ORDER BY created_at DESC, job_id DESC
            ) AS row_number
        FROM jobs
        WHERE kind = 'ingest'
    ) ranked
    WHERE row_number = 1
      AND status = 'failed'
), latest_url_import AS (
    SELECT group_id, job_id, kind, file_id, failure_stage, error_message, created_at
    FROM (
        SELECT
            jobs.*,
            ROW_NUMBER() OVER (
                PARTITION BY group_id, dedupe_key
                ORDER BY created_at DESC, job_id DESC
            ) AS row_number
        FROM jobs
        WHERE kind = 'url_import'
    ) ranked
    WHERE row_number = 1
      AND status = 'failed'
), candidates AS (
    SELECT group_id, job_id, kind, file_id, created_at
    FROM latest_ingest
    WHERE file_id IS NOT NULL
      AND ($3::TEXT IS NULL OR failure_stage = $3::TEXT)
      AND ($4::TEXT IS NULL OR COALESCE(error_message, '') ILIKE '%' || BTRIM($4::TEXT) || '%')

    UNION ALL

    SELECT group_id, job_id, kind, file_id, created_at
    FROM latest_url_import
    WHERE ($3::TEXT IS NULL OR failure_stage = $3::TEXT)
      AND ($4::TEXT IS NULL OR COALESCE(error_message, '') ILIKE '%' || BTRIM($4::TEXT) || '%')
)
SELECT
    group_id AS "group_id!",
    job_id AS "job_id!",
    kind AS "kind!",
    file_id,
    COUNT(*) OVER () AS "candidate_count!: i64"
FROM candidates
ORDER BY created_at, job_id
LIMIT $5
