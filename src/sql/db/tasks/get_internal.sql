-- `file_name` and `document_title` are the collapsed row's human-readable
-- context (issue 723): the focus item's file name and document title, with the
-- fallback chain documented in `items.sql`. One LATERAL row keeps the join from
-- multiplying a task, so the internal read returns the same shape as the
-- user-facing one.
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
FROM context69.tasks task
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
WHERE task.id = $1
