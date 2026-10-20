-- Promote a staged artifact object after its reference committed (issue #975 P3).
--
-- The staging lease is dropped only for an object that a committed artifact
-- reference already owns. A shared task-input staging lease is preserved: the
-- input reference still needs its 24-hour protection, and this object stays
-- owned by whoever releases it last.
UPDATE context69.library_storage_objects AS object
SET staging_lease_until = NULL,
    updated_at = now()
WHERE object.id = $1
  AND object.staging_lease_until IS NOT NULL
  AND EXISTS (
      SELECT 1
      FROM context69.task_item_artifacts AS artifact
      WHERE artifact.storage_object_id = object.id
  )
  AND NOT EXISTS (
      SELECT 1
      FROM context69.task_items AS item
      WHERE item.input_storage_object_id = object.id
  )
RETURNING object.id
