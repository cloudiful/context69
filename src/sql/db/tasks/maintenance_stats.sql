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
       )::BIGINT AS "expired_terminal_count!"
FROM context69.tasks
