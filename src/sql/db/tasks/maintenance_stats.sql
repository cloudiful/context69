SELECT COALESCE(count(*), 0)::BIGINT AS "total_count!",
       COALESCE(count(*) FILTER (WHERE status = 'queued'), 0)::BIGINT AS "queued_count!",
       COALESCE(count(*) FILTER (WHERE status = 'running'), 0)::BIGINT AS "running_count!",
       COALESCE(count(*) FILTER (WHERE status = 'waiting'), 0)::BIGINT AS "waiting_count!",
       COALESCE(count(*) FILTER (WHERE status = 'succeeded'), 0)::BIGINT AS "succeeded_count!",
       COALESCE(count(*) FILTER (WHERE status = 'failed'), 0)::BIGINT AS "failed_count!",
       COALESCE(count(*) FILTER (WHERE status = 'cancelled'), 0)::BIGINT AS "cancelled_count!",
       COALESCE(count(*) FILTER (WHERE status IN ('queued', 'running', 'waiting')), 0)::BIGINT AS "active_count!",
       COALESCE(
           (SELECT count(*)::BIGINT
            FROM context69.tasks expired
            WHERE expired.status IN ('succeeded', 'failed', 'cancelled')
              AND COALESCE(expired.finished_at, expired.updated_at) < $1),
           0
       )::BIGINT AS "expired_terminal_count!",
       COALESCE(
           (SELECT count(*)::BIGINT
            FROM context69.task_external_jobs job
            WHERE job.provider = 'docling'
              AND job.status = 'submitting'),
           0
       )::BIGINT AS "uncertain_submitting_count!",
       COALESCE(
           (SELECT count(*)::BIGINT
            FROM context69.task_external_jobs job
            JOIN context69.task_items item ON item.id = job.item_id
            JOIN context69.tasks task ON task.id = item.task_id
            WHERE job.provider = 'docling'
              AND job.status = 'submitting'
              AND job.remote_task_id LIKE 'submitting-%'
              AND job.submitted_at < now() - interval '30 minutes'
              AND item.status IN ('succeeded', 'failed', 'cancelled')
              AND task.status IN ('succeeded', 'failed', 'cancelled')),
           0
       )::BIGINT AS "quarantinable_submitting_count!",
       COALESCE(
           (SELECT count(*)::BIGINT
            FROM context69.task_external_jobs job
            WHERE job.provider = 'docling'
              AND job.status = 'orphaned'),
           0
       )::BIGINT AS "orphaned_external_job_count!",
       -- Read-only capacity signal (issue #129 phase 4): persisted Docling
       -- remote-slot ceiling. Falls back to the single-worker default (1)
       -- when the singleton settings row is missing; never tunes admission.
       COALESCE(
           (SELECT max_inflight::BIGINT
            FROM context69.docling_settings
            WHERE singleton = TRUE),
           1
       )::BIGINT AS "docling_max_inflight!",
       -- Read-only backpressure signal (issue #129 phase 4): admission-deferred
       -- items ready for retry. Only `waiting/backoff` rows carrying the
       -- persistent-admission denial marker
       -- (`docling_admission_denied` in task_ingest.rs) are counted; ordinary
       -- backoff, dependency, and external_job waits are excluded. `due` means
       -- the deferral delay has elapsed (`next_attempt_at <= now()`).
       COALESCE(
           (SELECT count(*)::BIGINT
            FROM context69.task_items item
            WHERE item.status = 'waiting'
              AND item.waiting_reason = 'backoff'
              AND item.error_message LIKE '%remote admission is full%'
              AND (item.next_attempt_at IS NULL OR item.next_attempt_at <= now())),
           0
       )::BIGINT AS "due_docling_waiting_count!",
       -- Read-only age signals (issue #129 phase 4): oldest uncertain
       -- `submitting` row and oldest quarantinable subset (same eligibility
       -- predicate as `quarantinable_submitting_count`: placeholder remote id,
       -- older than 30 minutes, terminal parents). NULL when the bucket is
       -- empty; never mutates state.
       (SELECT min(job.submitted_at)
        FROM context69.task_external_jobs job
        WHERE job.provider = 'docling'
          AND job.status = 'submitting') AS oldest_uncertain_submitting_at,
       (SELECT min(job.submitted_at)
        FROM context69.task_external_jobs job
        JOIN context69.task_items item ON item.id = job.item_id
        JOIN context69.tasks task ON task.id = item.task_id
        WHERE job.provider = 'docling'
          AND job.status = 'submitting'
          AND job.remote_task_id LIKE 'submitting-%'
          AND job.submitted_at < now() - interval '30 minutes'
          AND item.status IN ('succeeded', 'failed', 'cancelled')
          AND task.status IN ('succeeded', 'failed', 'cancelled')) AS oldest_quarantinable_submitting_at
FROM context69.tasks
