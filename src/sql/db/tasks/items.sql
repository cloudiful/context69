-- `file_name` and `document_title` are the human-readable context of one item
-- (issue 723). Both are projected from rows the item already owns, with a
-- deterministic fallback chain so a row is never blank:
--
--   file_name      library file name, else the filename the payload retained,
--                  else the last path segment of the submitted URL. A cancelled
--                  or not-yet-materialized item has no library file yet, so the
--                  retained payload is what still names it.
--   document_title the title the submission carried, else the title of the
--                  document this item's file produced (via
--                  `library_file_documents`), else the library file's own
--                  `metadata_json.title`.
--
-- Every join is 1:1 — `library_files` on its primary key, and the linked
-- document through one LATERAL row — so neither can multiply an item row. The
-- document comes from `library_file_documents`, the authoritative record of
-- which document an item's own file produced, so an item can never name a
-- document another file produced. Every candidate goes through `NULLIF`: an
-- empty string must fall through to the next source instead of masking it, and
-- it is also what makes the file name nullable, since a plain joined column
-- reference is described as NOT NULL.
--
-- `$4::text` narrows to one item status; NULL lists every status.
-- `$5::text` selects the ordering. `'ordinal'` returns the lowest ordinals
-- first, which is what `GET /v1/tasks/{task_id}/diagnose` needs: its contract
-- promises the first `limit` items of the task's sequence, so it must select
-- them before it truncates. Every other value keeps the fixed active-first
-- ordering the paged items endpoint documents.
-- `$6::uuid` narrows to one item; NULL lists every item of the task. This is
-- how a lifecycle log resolves one claimed item's ordinal without assuming
-- which item a claim picked.
-- Fixed active-first ordering (failed, running, queued, waiting,
-- cancelled, succeeded, then ordinal) pins in-flight/failed to the top and
-- sinks succeeded to the bottom. `cursor`/`offset` ($3) is scoped to the
-- current status filter: clients reset to 0 when the filter changes, then
-- follow `next_cursor` until NULL to page through the full filtered set.
SELECT item.id,
       item.task_id,
       item.ordinal,
       item.status,
       item.resource_id,
       item.file_id,
       item.stage,
       item.waiting_reason,
       item.dependency_key,
       item.next_attempt_at,
       item.failure_stage,
       item.error_message,
       item.attempt_count,
       item.retryable,
       item.created_at,
       item.started_at,
       item.finished_at,
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
-- The document a file produced is recorded per section, so a plain join would
-- fan the item row out. One LATERAL row picks the first section in document
-- order, which keeps the projection at one row per item and makes the choice
-- deterministic. The document must also belong to the group that owns the file,
-- so a title can never cross a group boundary.
LEFT JOIN LATERAL (
    SELECT document.title
    FROM context69.library_file_documents link
    JOIN context69.documents document ON document.id = link.document_id
    WHERE link.file_id = file.id
      AND document.group_id = file.group_id
    ORDER BY link.sort_order, link.section_key
    LIMIT 1
) doc ON TRUE
WHERE item.task_id = $1
  AND ($4::text IS NULL OR item.status = $4::text)
  AND ($6::uuid IS NULL OR item.id = $6::uuid)
ORDER BY
    CASE
        WHEN $5::text = 'ordinal' THEN 0
        ELSE CASE item.status
            WHEN 'failed' THEN 0
            WHEN 'running' THEN 1
            WHEN 'queued' THEN 2
            WHEN 'waiting' THEN 3
            WHEN 'cancelled' THEN 4
            WHEN 'succeeded' THEN 5
            ELSE 6
        END
    END,
    item.ordinal
LIMIT $2 OFFSET $3