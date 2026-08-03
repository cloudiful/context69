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
        count(*) FILTER (
            WHERE item.status = 'failed'
              AND item.updated_at >= now() - interval '1 hour'
        )::BIGINT AS recent_failure_count,
        count(*) FILTER (
            WHERE item.stage = 'docling'
              AND item.status IN ('queued', 'running', 'waiting')
        )::BIGINT AS docling_required_count
    FROM context69.task_items item
)
SELECT
    queue_counts.pending_count,
    queue_counts.queued_count,
    queue_counts.oldest_pending_at,
    queue_counts.oldest_queued_at,
    queue_counts.recent_failure_count,
    queue_counts.docling_required_count,
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
    recent_processing.processed_last_hour,
    recent_processing.failed_last_hour
FROM queue_counts
CROSS JOIN recent_processing
