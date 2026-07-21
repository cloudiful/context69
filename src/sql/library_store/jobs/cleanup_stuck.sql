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
), stale_urls AS (
    SELECT job.id, job.file_id, job.ingest_job_id, job.lease_token
    FROM context69.library_url_import_jobs job
    JOIN context69.groups group_row ON group_row.id = job.group_id
    WHERE (group_row.visibility = 'public' OR group_row.id = ANY($2::BIGINT[]))
      AND group_row.id IN (SELECT group_id FROM managed_groups)
      AND job.status IN ('downloading', 'ingesting')
      AND (
          job.lease_expires_at IS NOT NULL
          AND job.lease_expires_at < now()
      )
), stale_standalone_ingests AS (
    SELECT job.id, job.file_id
    FROM context69.library_ingest_jobs job
    JOIN context69.groups group_row ON group_row.id = job.group_id
    WHERE (group_row.visibility = 'public' OR group_row.id = ANY($2::BIGINT[]))
      AND group_row.id IN (SELECT group_id FROM managed_groups)
      AND job.status IN ('pending', 'running')
      AND job.updated_at < $3
      AND NOT EXISTS (
          SELECT 1
          FROM context69.library_url_import_jobs url_job
          WHERE url_job.ingest_job_id = job.id
      )
), updated_urls AS (
    UPDATE context69.library_url_import_jobs job
    SET status = 'failed',
        failure_stage = 'other',
        error_code = 'processing_timeout',
        error_message = $4,
        finished_at = now(),
        lease_token = NULL,
        lease_expires_at = NULL,
        updated_at = now()
    FROM stale_urls target
    WHERE job.id = target.id
      AND job.status IN ('downloading', 'ingesting')
      AND job.lease_token = target.lease_token
      AND job.lease_expires_at IS NOT NULL
      AND job.lease_expires_at < now()
    RETURNING job.id, job.file_id, job.ingest_job_id
), target_ingests AS (
    SELECT id, file_id, TRUE AS canonical
    FROM stale_standalone_ingests
    UNION ALL
    SELECT ingest.id, ingest.file_id, FALSE AS canonical
    FROM updated_urls url_job
    JOIN context69.library_ingest_jobs ingest ON ingest.id = url_job.ingest_job_id
    WHERE ingest.status IN ('pending', 'running')
), updated_ingests AS (
    UPDATE context69.library_ingest_jobs job
    SET status = 'failed',
        failure_stage = 'other',
        error_message = $4,
        finished_at = now(),
        updated_at = now()
    FROM target_ingests target
    WHERE job.id = target.id
      AND job.status IN ('pending', 'running')
    RETURNING job.id, job.file_id, target.canonical
), target_files AS (
    SELECT file_id FROM updated_ingests WHERE file_id IS NOT NULL
    UNION ALL
    SELECT file_id FROM updated_urls WHERE file_id IS NOT NULL
), updated_files AS (
    UPDATE context69.library_files file
    SET ingest_status = 'failed',
        error_message = $4,
        updated_at = now()
    FROM target_files target
    WHERE file.id = target.file_id
      AND file.ingest_status IN ('pending', 'running')
    RETURNING file.id
), action_targets AS (
    SELECT id FROM stale_urls
    UNION ALL
    SELECT id FROM stale_standalone_ingests
)
SELECT
    (
        SELECT COUNT(*) FROM updated_ingests WHERE canonical
    ) + (
        SELECT COUNT(*) FROM updated_urls
    ) AS "accepted!: i64",
    (
        SELECT COUNT(*) FROM action_targets
    ) - (
        (
            SELECT COUNT(*) FROM updated_ingests WHERE canonical
        ) + (
            SELECT COUNT(*) FROM updated_urls
        )
    ) AS "skipped!: i64"
