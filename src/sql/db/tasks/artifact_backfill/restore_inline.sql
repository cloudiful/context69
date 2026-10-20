-- Inverse CAS: restore one missing inline field from a verified artifact and
-- remove exactly that reference in the same statement (issue #975 P6).
--
-- The parent task row is locked before the item, matching the rerun and forward
-- apply order. The UPDATE rechecks the terminal item and parent, the absent
-- inline field, the non-NULL group and the exact registry identity; it restores
-- ONLY the missing field and preserves every other payload key, including
-- `indexing_checkpoint`. The reference DELETE is driven from the UPDATE's
-- RETURNING, so a restore and its reference removal always commit together, and
-- the P1 trigger enqueues the old-object cleanup in the same transaction.
--
-- Any mismatch (changed reference, new claim, restored inline value, other
-- group, inactive backend) yields zero rows and changes nothing.
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
), restored AS (
    UPDATE context69.task_items AS item
    SET payload = CASE WHEN $2::text = 'download'
                       THEN jsonb_set(
                                item.payload,
                                '{download_artifact,content_base64}',
                                to_jsonb($8::text),
                                true
                            )
                       ELSE jsonb_set(
                                item.payload,
                                '{section_payload}',
                                $9::jsonb,
                                true
                            )
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
          ($2::text = 'download' AND $7::text = 'raw-v1')
          OR ($2::text = 'sections' AND $7::text = 'ingest-sections-json-v1')
      )
      AND (
          CASE WHEN $2::text = 'download'
               THEN jsonb_typeof(item.payload -> 'download_artifact') = 'object'
                    AND item.payload #> '{download_artifact,content_base64}' IS NULL
               ELSE item.payload -> 'section_payload' IS NULL
          END
      )
      AND EXISTS (
          SELECT 1
          FROM context69.task_item_artifacts AS artifact
          WHERE artifact.item_id = item.id
            AND artifact.kind = $2
            AND artifact.storage_object_id = $3
            AND artifact.encoding = $7
      )
    RETURNING item.id
), removed AS (
    DELETE FROM context69.task_item_artifacts AS artifact
    USING restored
    WHERE artifact.item_id = restored.id
      AND artifact.kind = $2
      AND artifact.storage_object_id = $3
    RETURNING artifact.item_id
)
SELECT count(*)::bigint AS "restored!"
FROM removed
