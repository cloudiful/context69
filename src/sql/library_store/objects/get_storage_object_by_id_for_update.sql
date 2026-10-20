-- Re-read an orphan candidate under the identity lock (issue #975 P8).
--
-- Physical deletion happens after this guard, so an artifact reference that
-- appeared after enumeration must refuse the candidate here.
SELECT id, group_id, sha256, size_bytes, storage_backend, object_key, staging_lease_until
FROM context69.library_storage_objects AS object
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
FOR UPDATE
