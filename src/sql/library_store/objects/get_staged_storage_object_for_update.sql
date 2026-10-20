-- Lock a releasable staged object (issue #975 P8).
--
-- The artifact relation is checked here as well as the file and task-input
-- references: an object another item's artifact owns must never be admitted to
-- the physical-delete path, because the bytes are that checkpoint's data.
SELECT id, group_id, sha256, size_bytes, storage_backend, object_key, staging_lease_until
FROM context69.library_storage_objects AS object
WHERE object.id = $1
  AND object.staging_lease_until IS NOT NULL
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
FOR UPDATE
