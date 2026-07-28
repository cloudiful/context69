WITH next_job AS (
    SELECT job.id
    FROM context69.library_ingest_jobs job
    JOIN context69.library_files file ON file.id = job.file_id
    JOIN context69.library_dependency_gates embedding_gate
        ON embedding_gate.dependency_key = 'embedding_vector'
    LEFT JOIN context69.library_dependency_gates docling_gate
        ON docling_gate.dependency_key = 'docling'
    LEFT JOIN context69.library_dependency_gates s3_gate
        ON s3_gate.dependency_key = 's3'
    WHERE job.status = 'pending'
      AND NOT EXISTS (
          SELECT 1
          FROM context69.library_ingest_jobs running_job
          WHERE running_job.file_id = job.file_id
            AND running_job.status = 'running'
      )
      AND (
          embedding_gate.state = 'closed'
          OR embedding_gate.probe_lease_token = $1
      )
      AND (
          NOT job.requires_docling
          OR docling_gate.state = 'closed'
          OR docling_gate.probe_lease_token = $1
      )
      AND (
          $3::TEXT <> 's3'
          OR s3_gate.state = 'closed'
          OR s3_gate.probe_lease_token = $1
      )
    ORDER BY
        (
            job.requires_docling
            AND docling_gate.probe_lease_token = $1
        ) DESC,
        (
            $3::TEXT = 's3'
            AND s3_gate.probe_lease_token = $1
        ) DESC,
        job.created_at,
        job.id
    FOR UPDATE OF job, file SKIP LOCKED
    LIMIT 1
),
claimed AS (
    UPDATE context69.library_ingest_jobs job
SET status = 'running',
    lease_token = $1,
    lease_expires_at = now() + ($2::BIGINT * INTERVAL '1 second'),
    started_at = COALESCE(started_at, now()),
    finished_at = NULL,
    error_message = NULL,
    failure_stage = NULL,
    updated_at = now()
FROM next_job
WHERE job.id = next_job.id
    RETURNING job.id, job.file_id, job.requires_docling, job.lease_token, job.section_payload
)
SELECT
    claimed.id AS "job_id!",
    claimed.file_id AS "file_id!",
    job.created_at AS "created_at!",
    claimed.requires_docling AS "requires_docling!",
    claimed.lease_token AS "lease_token!",
    claimed.section_payload,
    $3::TEXT AS "storage_backend!"
FROM claimed
JOIN context69.library_ingest_jobs job ON job.id = claimed.id;
