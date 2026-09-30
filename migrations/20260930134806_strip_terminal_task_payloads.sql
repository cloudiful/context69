-- Bounded terminal task-payload retention (issue #667 Phase 2B).
--
-- The canonical bytes of a processed item live in the content-addressed
-- library storage object; a terminal item only needs the workflow metadata
-- that is still read by retry/rerun. Historical rows accumulated obsolete
-- large payload keys, so this migration strips them under explicit
-- kind/status/file guards. It never deletes a row, never touches
-- task/library/storage tables other than the payload column, and adds no
-- schema object.
--
-- Three independent, idempotent updates:
--
--   1. Succeeded `url_batch` items drop the obsolete top-level
--      `download_artifact` object (resolved source metadata and, for legacy
--      rows, the inline base64 bytes). Failed/cancelled URL items keep it so
--      a retry or rerun still has its input.
--   2. Terminal `text_batch`/`file_batch` items with a durable `file_id` drop
--      `section_payload` and `indexing_checkpoint`: both are reconstructed
--      from the canonical file object on the next re-drive.
--   3. Succeeded `text_batch` items whose library file durably reached
--      `ingest_status = 'succeeded'` drop the inline `content`. Failed and
--      cancelled text items keep `content` because their retry re-parses the
--      original text request.
--
-- Re-running the migration is a no-op: each statement matches only while the
-- stripped key still exists, so the second pass updates no row.
--
-- Rollback: none required — every removed key is reconstructible from the
-- canonical stored object, and no schema or row is changed.

UPDATE context69.task_items AS item
SET payload = item.payload - 'download_artifact',
    updated_at = now()
WHERE item.status = 'succeeded'
  AND item.file_id IS NOT NULL
  AND item.payload ? 'download_artifact'
  AND EXISTS (
      SELECT 1
      FROM context69.tasks AS task
      WHERE task.id = item.task_id
        AND task.kind = 'url_batch'
  );

UPDATE context69.task_items AS item
SET payload = item.payload - 'section_payload' - 'indexing_checkpoint',
    updated_at = now()
WHERE item.status IN ('succeeded', 'failed', 'cancelled')
  AND item.file_id IS NOT NULL
  AND (item.payload ? 'section_payload' OR item.payload ? 'indexing_checkpoint')
  AND EXISTS (
      SELECT 1
      FROM context69.tasks AS task
      WHERE task.id = item.task_id
        AND task.kind IN ('text_batch', 'file_batch')
  );

UPDATE context69.task_items AS item
SET payload = item.payload - 'content',
    updated_at = now()
WHERE item.status = 'succeeded'
  AND item.file_id IS NOT NULL
  AND item.payload ? 'content'
  AND EXISTS (
      SELECT 1
      FROM context69.tasks AS task
      JOIN context69.library_files AS file ON file.id = item.file_id
      WHERE task.id = item.task_id
        AND task.kind = 'text_batch'
        AND file.ingest_status = 'succeeded'
  );
