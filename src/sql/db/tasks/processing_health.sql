WITH status_counts AS (
    SELECT item.status AS key, count(*)::BIGINT AS count
    FROM context69.task_items item
    GROUP BY item.status
),
stage_counts AS (
    SELECT item.stage AS key, count(*)::BIGINT AS count
    FROM context69.task_items item
    WHERE item.status IN ('queued', 'running', 'waiting')
      AND item.stage IS NOT NULL
    GROUP BY item.stage
),
waiting_reason_counts AS (
    SELECT item.waiting_reason AS key, count(*)::BIGINT AS count
    FROM context69.task_items item
    WHERE item.status = 'waiting'
      AND item.waiting_reason IS NOT NULL
    GROUP BY item.waiting_reason
),
dependency_counts AS (
    SELECT item.dependency_key AS key, count(*)::BIGINT AS count
    FROM context69.task_items item
    WHERE item.status = 'waiting'
      AND item.dependency_key IS NOT NULL
    GROUP BY item.dependency_key
),
recent_processing AS (
    SELECT
        count(*) FILTER (
            WHERE item.status IN ('succeeded', 'failed', 'cancelled')
              AND item.updated_at >= now() - interval '1 hour'
        )::BIGINT AS processed_last_hour,
        count(*) FILTER (
            WHERE item.status = 'failed'
              AND item.updated_at >= now() - interval '1 hour'
        )::BIGINT AS failed_last_hour
    FROM context69.task_items item
),
queue_counts AS (
    SELECT
        count(*) FILTER (WHERE item.status IN ('queued', 'running', 'waiting'))::BIGINT AS pending_count,
        count(*) FILTER (WHERE item.status = 'queued')::BIGINT AS queued_count,
        min(item.created_at) FILTER (WHERE item.status IN ('queued', 'running', 'waiting')) AS oldest_pending_at,
        min(item.created_at) FILTER (WHERE item.status = 'queued') AS oldest_queued_at,
        min(item.waiting_since) FILTER (WHERE item.status = 'waiting') AS oldest_waiting_at,
        count(*) FILTER (
            WHERE item.status = 'failed'
              AND item.updated_at >= now() - interval '1 hour'
        )::BIGINT AS recent_failure_count,
        count(*) FILTER (
            WHERE item.stage IN ('docling', 'docling_poll')
              AND item.status IN ('queued', 'running', 'waiting')
        )::BIGINT AS docling_required_count,
        count(*) FILTER (
            WHERE item.status = 'waiting'
              AND item.waiting_reason = 'dependency'
              AND item.dependency_key = 'docling'
        )::BIGINT AS docling_dependency_waiting_count,
        count(*) FILTER (
            WHERE item.status = 'waiting'
              AND item.waiting_since < now() - interval '30 minutes'
        )::BIGINT AS stale_waiting_count
    FROM context69.task_items item
), external_jobs AS (
    SELECT
        count(*) FILTER (
            WHERE job.status IN ('submitting', 'pending', 'running')
              AND job.deadline_at IS NOT NULL
              AND job.deadline_at < now()
        )::BIGINT AS expired_active_jobs,
         count(*) FILTER (WHERE job.status IN ('submitting', 'pending', 'running'))::BIGINT AS active_jobs
    FROM context69.task_external_jobs job
)
SELECT
    queue_counts.pending_count AS "pending_count!",
    queue_counts.queued_count AS "queued_count!",
    queue_counts.oldest_pending_at,
    queue_counts.oldest_queued_at,
    queue_counts.oldest_waiting_at,
    queue_counts.recent_failure_count AS "recent_failure_count!",
    queue_counts.docling_required_count AS "docling_required_count!",
    queue_counts.docling_dependency_waiting_count AS "docling_dependency_waiting_count!",
    queue_counts.stale_waiting_count AS "stale_waiting_count!",
    external_jobs.expired_active_jobs AS "expired_active_jobs!",
    external_jobs.active_jobs AS "active_jobs!",
    COALESCE(
        (SELECT jsonb_agg(jsonb_build_object('key', key, 'count', count) ORDER BY key)
         FROM status_counts),
        '[]'::jsonb
    ) AS status_counts,
    COALESCE(
        (SELECT jsonb_agg(jsonb_build_object('key', key, 'count', count) ORDER BY key)
         FROM stage_counts),
        '[]'::jsonb
    ) AS stage_counts,
    COALESCE(
        (SELECT jsonb_agg(jsonb_build_object('key', key, 'count', count) ORDER BY key)
         FROM waiting_reason_counts),
        '[]'::jsonb
    ) AS waiting_reason_counts,
    COALESCE(
        (SELECT jsonb_agg(jsonb_build_object('key', key, 'count', count) ORDER BY key)
         FROM dependency_counts),
        '[]'::jsonb
    ) AS dependency_counts,
    recent_processing.processed_last_hour AS "processed_last_hour!",
    recent_processing.failed_last_hour AS "failed_last_hour!"
FROM queue_counts
CROSS JOIN recent_processing
CROSS JOIN external_jobs
