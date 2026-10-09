-- `file_name` and `document_title` are the collapsed row's human-readable
-- context (issue 723): the focus item's file name and document title, with the
-- fallback chain documented in `items.sql`. One LATERAL row keeps the join from
-- multiplying a task.
--
-- Issue 730 page-first: the filtered/sorted page of parent tasks is selected
-- in the `page` CTE before any filename/title lookup runs. The outer query
-- joins the focus-item/document LATERALs only against those page rows, so
-- metadata work is bounded by the page size. The count query stays
-- metadata-free on purpose.
--
-- Issue 734: the inherited accessible groups are computed once in
-- `accessible_groups` (scoped to the page selection, outside the
-- task-correlated EXISTS) instead of per task row. The page keeps an EXISTS
-- against that set (never a join), so overlapping memberships cannot duplicate
-- a task and ownership/descendant semantics match the original.
WITH page AS (
    WITH RECURSIVE accessible_groups AS (
        SELECT gm.group_id
        FROM context69.group_memberships gm
        WHERE gm.user_id = $1
        UNION ALL
        SELECT child.id
        FROM context69.groups child
        JOIN accessible_groups ON child.parent_group_id = accessible_groups.group_id
    )
    SELECT
        task.id,
        task.user_id,
        task.group_id,
        task.kind,
        task.status,
        task.origin,
        task.group_path,
        task.source_key,
        task.total_count,
        task.queued_count,
        task.running_count,
        task.waiting_count,
        task.succeeded_count,
        task.failed_count,
        task.cancelled_count,
        task.failure_stage,
        task.error_summary,
        task.stage,
        task.waiting_reason,
        task.dependency_key,
        task.next_attempt_at,
        task.deleted_at,
        task.created_at,
        task.started_at,
        task.finished_at,
        task.updated_at
    FROM context69.tasks task
    WHERE (
          task.user_id = $1
          OR EXISTS (
              SELECT 1
              FROM accessible_groups ag
              WHERE ag.group_id = task.group_id
          )
      )
      AND (
          $2::text IS NULL
          OR task.group_path ILIKE '%' || $2 || '%'
          OR task.source_key ILIKE '%' || $2 || '%'
          OR task.error_summary ILIKE '%' || $2 || '%'
          OR EXISTS (
              SELECT 1
              FROM context69.task_items item
              WHERE item.task_id = task.id
                AND item.payload::text ILIKE '%' || $2 || '%'
          )
      )
      AND ($3::text IS NULL OR task.kind = $3)
      AND ($4::text IS NULL OR task.status = $4)
      AND ($5::text IS NULL OR task.stage = $5)
      AND ($6::text IS NULL OR task.waiting_reason = $6)
      AND ($7::text IS NULL OR task.dependency_key = $7)
      AND (
          ($12::text = 'processing' AND task.deleted_at IS NULL AND task.status <> 'succeeded')
          OR ($12::text = 'completed' AND task.deleted_at IS NULL AND task.status = 'succeeded')
          OR ($12::text = 'trash' AND task.deleted_at IS NOT NULL)
      )
    ORDER BY
        CASE WHEN $8::TEXT = 'status' AND $9::TEXT = 'asc' THEN task.status END ASC NULLS LAST,
        CASE WHEN $8::TEXT = 'status' AND $9::TEXT = 'desc' THEN task.status END DESC NULLS LAST,
        CASE WHEN $8::TEXT = 'kind' AND $9::TEXT = 'asc' THEN task.kind END ASC NULLS LAST,
        CASE WHEN $8::TEXT = 'kind' AND $9::TEXT = 'desc' THEN task.kind END DESC NULLS LAST,
        CASE WHEN $8::TEXT = 'stage' AND $9::TEXT = 'asc' THEN COALESCE(task.stage, '') END ASC NULLS LAST,
        CASE WHEN $8::TEXT = 'stage' AND $9::TEXT = 'desc' THEN COALESCE(task.stage, '') END DESC NULLS LAST,
        CASE WHEN $8::TEXT = 'group_path' AND $9::TEXT = 'asc' THEN COALESCE(task.group_path, '') END ASC NULLS LAST,
        CASE WHEN $8::TEXT = 'group_path' AND $9::TEXT = 'desc' THEN COALESCE(task.group_path, '') END DESC NULLS LAST,
        CASE WHEN $8::TEXT = 'created_at' AND $9::TEXT = 'asc' THEN task.created_at END ASC NULLS LAST,
        CASE WHEN $8::TEXT = 'created_at' AND $9::TEXT = 'desc' THEN task.created_at END DESC NULLS LAST,
        CASE WHEN $8::TEXT = 'updated_at' AND $9::TEXT = 'asc' THEN task.updated_at END ASC NULLS LAST,
        CASE WHEN $8::TEXT = 'updated_at' AND $9::TEXT = 'desc' THEN task.updated_at END DESC NULLS LAST,
        task.created_at DESC,
        task.id DESC
    LIMIT $10 OFFSET $11
)
SELECT
    task.id,
    task.user_id,
    task.group_id,
    task.kind,
    task.status,
    task.origin,
    task.group_path,
    task.source_key,
    task.total_count,
    task.queued_count,
    task.running_count,
    task.waiting_count,
    task.succeeded_count,
    task.failed_count,
    task.cancelled_count,
    task.failure_stage,
    task.error_summary,
    task.stage,
    task.waiting_reason,
    task.dependency_key,
    task.next_attempt_at,
    task.deleted_at,
    task.created_at,
    task.started_at,
    task.finished_at,
    task.updated_at,
    context.file_name,
    context.document_title
FROM page task
LEFT JOIN LATERAL (
    SELECT
        -- The focus item's readable context; `items.sql` documents the same
        -- fallback chain for one item.
        COALESCE(
            NULLIF(file.filename, ''),
            NULLIF(item.payload ->> 'filename', ''),
            CASE
                WHEN item.payload ? 'url'
                    THEN NULLIF(
                        regexp_replace(
                            split_part(split_part(item.payload ->> 'url', '?', 1), '#', 1),
                            '^.*/',
                            ''
                        ),
                        ''
                    )
            END
        ) AS file_name,
        COALESCE(
            NULLIF(item.payload ->> 'title', ''),
            NULLIF(doc.title, ''),
            NULLIF(file.metadata_json ->> 'title', '')
        ) AS document_title
    FROM context69.task_items item
    LEFT JOIN context69.library_files file ON file.id = item.file_id
    -- The document a file produced is recorded per section, so a plain join
    -- would fan the row out. One LATERAL row picks the first section in
    -- document order, which keeps the projection at one row per item and makes
    -- the choice deterministic. The document must also belong to the group that
    -- owns the file, so a title can never cross a group boundary.
    LEFT JOIN LATERAL (
        SELECT document.title
        FROM context69.library_file_documents link
        JOIN context69.documents document ON document.id = link.document_id
        WHERE link.file_id = file.id
          AND document.group_id = file.group_id
        ORDER BY link.sort_order, link.section_key
        LIMIT 1
    ) doc ON TRUE
    WHERE item.task_id = task.id
    -- The focus item is the lowest-ordinal item that is not yet succeeded, else
    -- the first item of an all-succeeded task: the item a user is looking at
    -- when the queue shows the row. One LATERAL row, so the join can never
    -- multiply a task.
    ORDER BY item.status = 'succeeded', item.ordinal
    LIMIT 1
) context ON TRUE
ORDER BY
    CASE WHEN $8::TEXT = 'status' AND $9::TEXT = 'asc' THEN task.status END ASC NULLS LAST,
    CASE WHEN $8::TEXT = 'status' AND $9::TEXT = 'desc' THEN task.status END DESC NULLS LAST,
    CASE WHEN $8::TEXT = 'kind' AND $9::TEXT = 'asc' THEN task.kind END ASC NULLS LAST,
    CASE WHEN $8::TEXT = 'kind' AND $9::TEXT = 'desc' THEN task.kind END DESC NULLS LAST,
    CASE WHEN $8::TEXT = 'stage' AND $9::TEXT = 'asc' THEN COALESCE(task.stage, '') END ASC NULLS LAST,
    CASE WHEN $8::TEXT = 'stage' AND $9::TEXT = 'desc' THEN COALESCE(task.stage, '') END DESC NULLS LAST,
    CASE WHEN $8::TEXT = 'group_path' AND $9::TEXT = 'asc' THEN COALESCE(task.group_path, '') END ASC NULLS LAST,
    CASE WHEN $8::TEXT = 'group_path' AND $9::TEXT = 'desc' THEN COALESCE(task.group_path, '') END DESC NULLS LAST,
    CASE WHEN $8::TEXT = 'created_at' AND $9::TEXT = 'asc' THEN task.created_at END ASC NULLS LAST,
    CASE WHEN $8::TEXT = 'created_at' AND $9::TEXT = 'desc' THEN task.created_at END DESC NULLS LAST,
    CASE WHEN $8::TEXT = 'updated_at' AND $9::TEXT = 'asc' THEN task.updated_at END ASC NULLS LAST,
    CASE WHEN $8::TEXT = 'updated_at' AND $9::TEXT = 'desc' THEN task.updated_at END DESC NULLS LAST,
    task.created_at DESC,
    task.id DESC
