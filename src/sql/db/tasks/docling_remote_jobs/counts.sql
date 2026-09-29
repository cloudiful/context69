-- Sweep observability for durable Docling remote jobs.
--
-- Single-row snapshot: active jobs, due jobs ready to claim (lease free,
-- deadline in the future), in-flight jobs holding a lease, expired jobs
-- past their deadline, and the queue age of the oldest due job in seconds.
-- No payload or secret columns are returned.
SELECT
    count(*) FILTER (WHERE status IN ('pending', 'running'))::BIGINT AS "active_count!",
    count(*) FILTER (
        WHERE status IN ('pending', 'running')
          AND next_poll_at <= now()
          AND (lease_until IS NULL OR lease_until < now())
          AND (deadline_at IS NULL OR deadline_at > now())
    )::BIGINT AS "due_count!",
    count(*) FILTER (
        WHERE status IN ('pending', 'running')
          AND lease_until IS NOT NULL AND lease_until >= now()
    )::BIGINT AS "inflight_count!",
    count(*) FILTER (
        WHERE status IN ('pending', 'running')
          AND deadline_at IS NOT NULL AND deadline_at <= now()
    )::BIGINT AS "expired_count!",
    COALESCE(
        EXTRACT(
            EPOCH FROM (
                now() - MIN(next_poll_at) FILTER (
                    WHERE status IN ('pending', 'running')
                      AND next_poll_at <= now()
                      AND (lease_until IS NULL OR lease_until < now())
                      AND (deadline_at IS NULL OR deadline_at > now())
                )
            )
        )::BIGINT,
        0
    ) AS "oldest_due_age_secs!"
FROM context69.task_docling_remote_jobs
