SELECT EXISTS (
    SELECT 1 FROM context69.task_docling_remote_cleanup cleanup
    WHERE cleanup.id = $1 AND cleanup.claim_token = $2
      AND cleanup.claim_until > now()
      AND NOT EXISTS (
          SELECT 1 FROM context69.task_docling_remote_jobs job
          WHERE job.remote_task_id = cleanup.remote_task_id
            AND job.endpoint_base_url IS NOT DISTINCT FROM cleanup.endpoint_base_url
            AND job.status IN ('pending', 'running')
      )
) AS "eligible!";
