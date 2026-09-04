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
       )::BIGINT AS "orphaned_external_job_count!"
FROM context69.tasks
