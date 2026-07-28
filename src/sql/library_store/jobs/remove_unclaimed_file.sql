WITH candidate AS (
    SELECT
        job.id AS job_id,
        file.id AS file_id
    FROM context69.library_ingest_jobs job
    JOIN context69.library_files file ON file.id = job.file_id
    WHERE job.id = $2
      AND file.id = $1
      AND job.status = 'pending'
      AND job.lease_token IS NULL
      AND NOT EXISTS (
          SELECT 1
          FROM context69.library_ingest_jobs other_job
          WHERE other_job.file_id = file.id
            AND other_job.id <> job.id
      )
    FOR UPDATE OF job, file
), deleted_job AS (
    DELETE FROM context69.library_ingest_jobs job
    USING candidate
    WHERE job.id = candidate.job_id
    RETURNING job.file_id
)
DELETE FROM context69.library_files file
USING deleted_job
WHERE file.id = deleted_job.file_id
RETURNING file.storage_object_id, file.storage_rel_path
