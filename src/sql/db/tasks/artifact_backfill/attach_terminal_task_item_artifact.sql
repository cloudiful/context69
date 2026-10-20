-- Terminal-item artifact attach with an exact inline-field fingerprint
-- (issue #975 P5).
--
-- The running-item attach cannot serve a finished item: it requires a live
-- lease and a running status. This statement keeps the same one-statement
-- guarantee for terminal rows:
--   1. the parent task row is locked first, then the item row, matching the
--      rerun lock order;
--   2. the UPDATE rechecks the terminal item, the terminal parent, no lease, a
--      non-NULL group, the verified registry identity and the exact SHA-256 of
--      the current inline field text;
--   3. it removes ONLY the migrated field, preserving identity and checkpoint
--      keys, and inserts the reference from the UPDATE's RETURNING;
--   4. the reference insert takes the object key share lock, so the cleanup
--      worker cannot release the bytes between the reference commit and the
--      strip.
--
-- Any mismatch produces zero rows and changes nothing.
WITH locked_parent AS MATERIALIZED (
    SELECT task.id
    FROM context69.tasks AS task
    WHERE task.id = (
        SELECT item.task_id
        FROM context69.task_items AS item
        WHERE item.id = $1
    )
    FOR UPDATE
), locked_item AS MATERIALIZED (
    SELECT item.id
    FROM context69.task_items AS item
    JOIN locked_parent AS parent ON parent.id = item.task_id
    WHERE item.id = $1
    FOR UPDATE OF item
), attached_item AS (
    UPDATE context69.task_items AS item
    SET payload = CASE WHEN $2::text = 'download'
                       THEN item.payload #- '{download_artifact,content_base64}'
                       ELSE item.payload - 'section_payload'
                  END,
        updated_at = now()
    FROM context69.tasks AS task,
         context69.library_storage_objects AS object,
         locked_item
    WHERE item.id = locked_item.id
      AND task.id = item.task_id
      AND object.id = $3
      AND item.status IN ('succeeded', 'failed', 'cancelled')
      AND item.lease_token IS NULL
      AND item.lease_until IS NULL
      AND task.status IN ('succeeded', 'failed', 'cancelled')
      AND task.deleted_at IS NULL
      AND task.group_id IS NOT NULL
      AND object.group_id = task.group_id
      AND object.sha256 = $4
      AND object.size_bytes = $5
      AND object.storage_backend = $6
      AND (
          CASE WHEN $2::text = 'download'
               THEN item.payload #>> '{download_artifact,content_base64}'
               ELSE item.payload ->> 'section_payload'
          END
      ) IS NOT NULL
      AND encode(
              sha256(convert_to(
                  CASE WHEN $2::text = 'download'
                       THEN item.payload #>> '{download_artifact,content_base64}'
                       ELSE item.payload ->> 'section_payload'
                  END,
                  'UTF8'
              )),
              'hex'
          ) = $7
    RETURNING item.id
), attached_ref AS (
    INSERT INTO context69.task_item_artifacts
        (item_id, kind, storage_object_id, encoding)
    SELECT attached_item.id, $2::text, $3, $8::text
    FROM attached_item
    ON CONFLICT (item_id, kind) DO UPDATE
    SET storage_object_id = EXCLUDED.storage_object_id
    RETURNING item_id
)
SELECT count(*)::bigint AS "attached!"
FROM attached_ref
