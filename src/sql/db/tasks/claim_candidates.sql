-- The bounded, class-representative candidates the dispatcher may claim now
-- (issue 1020 P2, issue 1172 P2/P6A2).
--
-- The domain policy selects the lowest ordinal non-terminal item of an active
-- parent, with the retry time, Docling exclusion and execution failure budget
-- unchanged. Execution capacity is not decided here: the library attempt
-- admission is the only capacity owner.
--
-- A candidate must be linked to the real current library task of the queue.
-- The library snapshot is authoritative: a QUEUED segment is only admissible
-- when its immutable class equals the class its typed parent kind maps to and
-- its immutable resources contain that class and the shared `ingest` resource.
-- A NULL, unknown, mismatched or resource-incomplete queued segment is never
-- granted; the unclassified report counts it instead.
--
-- A CURRENT TERMINAL segment is different: its old metadata may predate the
-- class contract. It is admitted only for the existing bounded promotion, which
-- produces the next generation with the typed class and resources and never
-- claims or rewrites the terminal segment itself.
--
-- At most 333 oldest eligible heads are returned for each of the three
-- immutable classes, ordered by the parent creation time and the item id
-- within the class. That bounds one read at 999 candidates while keeping a
-- minority class represented when it has eligible work. The SQL does not
-- choose the winner: the library receives the candidate ids and applies its
-- own fair, capacity-aware admission.
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
    SELECT item.id, item.task_id, task.created_at, task.kind
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
    SELECT chosen.id,
           chosen.task_id,
           chosen.created_at,
           chosen.kind,
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
),
admissible AS (
    SELECT id,
           task_id,
           created_at,
           kind,
           segment_status,
           expected_class,
           CASE
               WHEN segment_status = 'queued' THEN segment_class
               ELSE expected_class
           END AS bound_class
    FROM linked
    WHERE expected_class IS NOT NULL
      AND (
            (
                segment_status = 'queued'
                AND segment_class = expected_class
                AND segment_resources @> ARRAY[segment_class, 'ingest']
            )
            OR segment_status IN ('succeeded', 'failed', 'cancelled')
        )
),
ranked AS (
    SELECT id,
           task_id,
           kind,
           segment_status,
           expected_class,
           ROW_NUMBER() OVER (
               PARTITION BY bound_class
               ORDER BY created_at, id
           ) AS class_rank
    FROM admissible
)
SELECT item.id,
       item.task_id,
       item.execution_task_id,
       item.execution_generation,
       ranked.segment_status AS "segment_status?",
       ranked.kind,
       ranked.expected_class AS "expected_class!",
       task.group_id,
       task.group_path,
       task.source_key
FROM ranked
JOIN context69.task_items item ON item.id = ranked.id
JOIN context69.tasks task ON task.id = ranked.task_id
WHERE ranked.class_rank <= 333
FOR UPDATE OF item SKIP LOCKED
