-- Delete an orphaned object record under the same guards as its under-lock
-- re-read (issue #975 P8), including artifact ownership.
DELETE FROM context69.library_storage_objects AS object
WHERE object.id = $1
  AND object.staging_lease_until IS NOT NULL
  AND object.staging_lease_until < $2
  AND NOT EXISTS (
      SELECT 1
      FROM context69.library_files AS file
      WHERE file.storage_object_id = object.id
  )
  AND NOT EXISTS (
      SELECT 1
      FROM context69.task_items AS item
      WHERE item.input_storage_object_id = object.id
  )
  AND NOT EXISTS (
      SELECT 1
      FROM context69.task_item_artifacts AS artifact
      WHERE artifact.storage_object_id = object.id
  )
RETURNING object.id
