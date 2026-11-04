-- Park one Docling submission as a durable unknown outcome (issue 1172 P5A).
--
-- `submission_uncertain` is the fail-closed state: the dispatch was invoked
-- and its outcome is unknown, so no automatic POST may ever resend it. Known
-- `pending`/`running` rows are never overwritten. A stored intent is resolved
-- by the captured owner that created it, or by a new captured owner once the
-- stored execution is no longer the item's running current segment.
--
-- `$1` job, `$2` item, `$3` sanitized reason, `$4` caller library task and
-- `$5` caller execution generation.
UPDATE context69.task_docling_remote_jobs AS job
SET status = 'submission_uncertain',
    remote_status = COALESCE(job.remote_status, job.status),
    last_error = COALESCE($3, job.last_error, 'docling dispatch outcome is unknown'),
    lease_token = NULL,
    lease_until = NULL,
    updated_at = clock_timestamp()
WHERE job.id = $1
  AND job.item_id = $2
  AND job.status IN ('submitting', 'submission_uncertain')
  AND (
      (
          job.submission_execution_task_id = $4
          AND job.submission_generation = $5
      )
      OR NOT EXISTS (
          SELECT 1
          FROM context69.task_items AS item
          WHERE item.id = $2
            AND item.status = 'running'
            AND item.execution_task_id = job.submission_execution_task_id
            AND item.execution_generation = job.submission_generation
      )
  )
RETURNING job.id
