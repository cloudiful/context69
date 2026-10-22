-- Lease-fenced artifact attach (issue #975 P3).
--
-- One statement owns the whole handoff:
--   1. the UPDATE locks and guards the running item under the caller's lease,
--      so a reclaim, cancel or finish that already happened cannot be stripped;
--   2. the same branch verifies the registry object is the caller's declared
--      identity in the item's own task group and on the active backend;
--   3. it removes ONLY the migrated inline bytes for that kind, leaving every
--      other payload key untouched (`#-`/`-` on a missing key are no-ops);
--   4. the INSERT upserts the artifact reference from the UPDATE's RETURNING,
--      so no reference without the matching strip and no strip without a
--      reference can commit.
--
-- The insert's foreign key takes FOR KEY SHARE on the storage-object row, which
-- conflicts with the cleanup worker's FOR UPDATE identity lock, so bytes can
-- never be released between this attach and its reference commit. A retarget
-- fires the P1 old-object release trigger; an unchanged object is a no-op.
--
-- The lease of the item fences the attach: the fencing token of the library
-- attempt and the deadline of that lease, compared against the clock of the
-- database.
WITH attached_item AS (
    UPDATE context69.task_items AS item
    SET payload = CASE WHEN $7::text = 'download'
                       THEN item.payload #- '{download_artifact,content_base64}'
                       ELSE item.payload - 'section_payload'
                  END,
        updated_at = now()
    FROM context69.tasks AS task
    WHERE item.id = $1
      AND item.lease_token = $2
      AND item.status = 'running'
      AND item.lease_until > clock_timestamp()
      AND task.id = item.task_id
      AND task.group_id IS NOT NULL
      AND EXISTS (
          SELECT 1
          FROM context69.library_storage_objects AS object
          WHERE object.id = $3
            AND object.group_id = task.group_id
            AND object.sha256 = $4
            AND object.size_bytes = $5
            AND object.storage_backend = $6
      )
    RETURNING item.id
), attached_ref AS (
    INSERT INTO context69.task_item_artifacts
        (item_id, kind, storage_object_id, encoding)
    SELECT attached_item.id, $7::text, $3, $8::text
    FROM attached_item
    ON CONFLICT (item_id, kind) DO UPDATE
    SET storage_object_id = EXCLUDED.storage_object_id
    RETURNING item_id
)
SELECT count(*)::bigint AS "attached!"
FROM attached_ref
