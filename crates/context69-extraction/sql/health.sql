SELECT
    COUNT(*) FILTER (WHERE status = 'queued' AND (next_attempt_at IS NULL OR next_attempt_at <= now()))::bigint AS queued,
    COUNT(*) FILTER (WHERE status = 'running')::bigint AS running,
    COUNT(*) FILTER (WHERE status = 'queued' AND next_attempt_at > now())::bigint AS awaiting_retry,
    MIN(next_attempt_at) FILTER (WHERE status = 'queued' AND next_attempt_at > now()) AS next_retry_at,
    COUNT(*) FILTER (WHERE status = 'failed' AND finished_at > now() - interval '1 hour')::bigint AS failed_last_hour,
    COALESCE(
        (SELECT jsonb_object_agg(failure_class, cnt)
         FROM (
             SELECT failure_class, COUNT(*)::bigint AS cnt
             FROM context69.document_extraction_jobs
             WHERE status = 'failed' AND failure_class IS NOT NULL
               AND finished_at > now() - interval '1 hour'
             GROUP BY failure_class
         ) sub),
        '{}'::jsonb
    ) AS failure_class_counts
FROM context69.document_extraction_jobs
