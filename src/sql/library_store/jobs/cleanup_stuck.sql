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
      AND (
          (
              job.status IN ('downloading', 'ingesting')
              AND job.lease_expires_at IS NOT NULL
              AND job.lease_expires_at < now()
          ) OR (
              job.status = 'downloading'
              AND job.lease_token IS NULL
              AND job.updated_at < $3
          )
      )
    FOR UPDATE OF job SKIP LOCKED
), stale_ingests AS (
    SELECT job.id, job.file_id, job.lease_token
    FROM context69.library_ingest_jobs job
    JOIN context69.groups group_row ON group_row.id = job.group_id
    WHERE (group_row.visibility = 'public' OR group_row.id = ANY($2::BIGINT[]))
      AND group_row.id IN (SELECT group_id FROM managed_groups)
      AND job.status = 'running'
      AND (
          (
              job.lease_expires_at IS NOT NULL
              AND job.lease_expires_at < now()
          ) OR (
              job.lease_token IS NULL
              AND job.updated_at < $3
          )
      )
    FOR UPDATE OF job SKIP LOCKED
), requeued_urls AS (
    UPDATE context69.library_url_import_jobs job
    SET status = CASE
            WHEN job.status = 'ingesting' AND job.ingest_job_id IS NOT NULL THEN 'ingesting'
            ELSE 'queued'
        END,
        next_attempt_at = CASE
            WHEN job.status = 'ingesting' AND job.ingest_job_id IS NOT NULL THEN NULL
            ELSE now() + INTERVAL '30 seconds'
        END,
        error_code = NULL,
        error_message = NULL,
        failure_stage = NULL,
        finished_at = NULL,
        lease_token = NULL,
        lease_expires_at = NULL,
        updated_at = now()
    FROM stale_urls target
    WHERE job.id = target.id
      AND (
          job.lease_token IS NOT DISTINCT FROM target.lease_token
          OR target.lease_token IS NULL
      )
    RETURNING job.id, job.file_id
), requeued_ingests AS (
    UPDATE context69.library_ingest_jobs job
    SET status = 'pending',
        lease_token = NULL,
        lease_expires_at = NULL,
        started_at = NULL,
        finished_at = NULL,
        failure_stage = NULL,
        error_message = NULL,
        updated_at = now()
    FROM stale_ingests target
    WHERE job.id = target.id
      AND (
          job.lease_token IS NOT DISTINCT FROM target.lease_token
          OR target.lease_token IS NULL
      )
    RETURNING job.id, job.file_id
), affected_files AS (
    SELECT file_id FROM requeued_urls WHERE file_id IS NOT NULL
    UNION ALL
    SELECT file_id FROM requeued_ingests WHERE file_id IS NOT NULL
), updated_files AS (
    UPDATE context69.library_files file
    SET ingest_status = 'pending',
        error_message = NULL,
        ingested_at = NULL,
        updated_at = now()
    FROM affected_files target
    WHERE file.id = target.file_id
      AND file.ingest_status IN ('pending', 'running')
    RETURNING file.id
), action_targets AS (
    SELECT id FROM stale_urls
    UNION ALL
    SELECT id FROM stale_ingests
)
SELECT
    (
        (SELECT COUNT(*) FROM requeued_urls)
        + (SELECT COUNT(*) FROM requeued_ingests)
    ) AS "accepted!: i64",
    (
        SELECT COUNT(*) FROM action_targets
    ) - (
        (SELECT COUNT(*) FROM requeued_urls)
        + (SELECT COUNT(*) FROM requeued_ingests)
    ) AS "skipped!: i64"
