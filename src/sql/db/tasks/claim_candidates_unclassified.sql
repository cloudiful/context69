-- The bounded count of eligible current queued heads whose immutable library
-- metadata fails the fail-closed admission contract (issue 1172 P6A2).
--
-- It is the exact complement of the queued branch of the claim candidate
-- query: an item is reported when its linked current segment is still queued
-- and its class is missing, unknown or different from the class its typed
-- parent kind maps to, or when its immutable resources do not contain both that
-- class and the shared `ingest` resource. The comparison uses SQL NULL
-- semantics so a NULL class is reported, not granted.
--
-- A terminal segment is not reported here: it takes the separate bounded
-- terminal promotion. A missing or dangling library pointer stays with the
-- separate adoption diagnostic and is never counted or claimed as queued work.
WITH current_items AS (
    SELECT DISTINCT ON (item.task_id) item.id, item.task_id, item.ordinal
    FROM context69.task_items item
    WHERE item.status IN ('queued', 'running', 'waiting')
    ORDER BY item.task_id, item.ordinal
),
eligible_items AS (
    SELECT item.id, item.task_id, item.ordinal
    FROM current_items item
    JOIN context69.task_items ti ON ti.id = item.id
    WHERE ti.status IN ('queued', 'waiting')
      AND ti.execution_failure_count < 5
      AND (
            (ti.next_attempt_at IS NULL OR ti.next_attempt_at <= now())
            OR (
                ti.status = 'waiting'
                AND COALESCE(ti.waiting_reason, '') = 'docling'
                AND EXISTS (
                    SELECT 1
                    FROM context69.task_docling_remote_jobs AS job
                    WHERE job.item_id = ti.id
                      AND job.status IN ('pending', 'running')
                      AND job.endpoint_base_url IS NOT NULL
                      AND (
                          job.next_poll_at <= now()
                          OR (job.deadline_at IS NOT NULL AND job.deadline_at <= now())
                      )
                )
            )
        )
      AND (
            NOT (
                ti.status = 'waiting'
                AND COALESCE(ti.waiting_reason, '') = 'docling'
            )
            OR EXISTS (
                SELECT 1
                FROM context69.task_docling_remote_jobs AS job
                WHERE job.item_id = ti.id
                  AND job.status IN ('pending', 'running')
                  AND job.endpoint_base_url IS NOT NULL
            )
        )
),
chosen AS (
    SELECT item.id, item.task_id, task.kind
    FROM eligible_items item
    JOIN context69.tasks task ON task.id = item.task_id
    WHERE task.status IN ('queued', 'running', 'waiting')
      AND task.deleted_at IS NULL
      AND (
            task.next_attempt_at IS NULL
            OR task.next_attempt_at <= now()
            OR EXISTS (
                SELECT 1
                FROM context69.task_docling_remote_jobs AS job
                WHERE job.item_id = item.id
                  AND job.status IN ('pending', 'running')
                  AND job.endpoint_base_url IS NOT NULL
                  AND (
                      job.next_poll_at <= now()
                      OR (job.deadline_at IS NOT NULL AND job.deadline_at <= now())
                  )
            )
        )
),
linked AS (
    SELECT chosen.kind,
           segment.status AS segment_status,
           segment.class AS segment_class,
           segment.resources AS segment_resources,
           CASE chosen.kind
               WHEN 'text_batch' THEN 'interactive'
               WHEN 'file_batch' THEN 'bulk'
               WHEN 'url_batch' THEN 'bulk'
               WHEN 'source_sync' THEN 'maintenance'
               WHEN 'delete_batch' THEN 'maintenance'
               WHEN 'translation' THEN 'maintenance'
               WHEN 'vector_rebuild' THEN 'maintenance'
               WHEN 'git_index' THEN 'maintenance'
           END AS expected_class
    FROM chosen
    JOIN context69.task_items item_row ON item_row.id = chosen.id
    JOIN cloudiful_tasks.tasks segment
      ON segment.id = item_row.execution_task_id
     AND segment.namespace = 'context69'
     AND segment.queue = 'items'
)
SELECT COUNT(*) AS "count!"
FROM linked
WHERE segment_status = 'queued'
  AND (
        expected_class IS NULL
        OR segment_class IS DISTINCT FROM expected_class
        OR NOT (
            segment_resources @> ARRAY[segment_class, 'ingest']
        )
    )
