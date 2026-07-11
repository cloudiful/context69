DELETE FROM context69.library_storage_objects AS object
WHERE object.id = $1
  AND NOT EXISTS (
      SELECT 1
      FROM context69.library_files AS file
      WHERE file.storage_object_id = object.id
  )
RETURNING object_key, storage_backend
