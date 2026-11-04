-- Bounded batch task read (issue 1172 P1).
--
-- One indexed set query (`task.id = ANY($1)`) replaces a per-id lookup loop, so
-- a 100-id caller lookup is one round trip and never an N+1. The projection and
-- the authorization predicate are the projection and the predicate of `get.sql`:
-- owner or a member of an inheriting group, with the same recursive membership
-- expansion. A row the caller may not read is simply absent from the result, so
-- the service can report `not_found` without leaking whether it exists.
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
    context.document_title,
    EXISTS (
        SELECT 1
        FROM context69.task_docling_remote_jobs job
        WHERE job.task_id = task.id
          AND job.status IN ('submitting', 'submission_uncertain')
    ) AS "docling_submission_uncertain!"
FROM context69.tasks task
LEFT JOIN LATERAL (
    SELECT
        -- The focus item's readable context; `get.sql` and `items.sql`
        -- document the same fallback chain for one task.
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
    -- One LATERAL row picks the first document section in document order, so
    -- the projection stays one row per task and never fans out.
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
    ORDER BY item.status = 'succeeded', item.ordinal
    LIMIT 1
) context ON TRUE
WHERE task.id = ANY($1::uuid[])
  AND (
      task.user_id = $2
      OR EXISTS (
          WITH RECURSIVE inherited_groups AS (
              SELECT gm.group_id
              FROM context69.group_memberships gm
              WHERE gm.user_id = $2
              UNION ALL
              SELECT child.id
              FROM context69.groups child
              JOIN inherited_groups ON child.parent_group_id = inherited_groups.group_id
          )
          SELECT 1
          FROM inherited_groups
          WHERE inherited_groups.group_id = task.group_id
      )
  )
