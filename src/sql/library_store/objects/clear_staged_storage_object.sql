UPDATE context69.library_storage_objects
SET staging_lease_until = NULL,
    updated_at = now()
WHERE id = $1
  AND EXISTS (
      SELECT 1
      FROM context69.library_files AS file
      WHERE file.id = $2
        AND file.storage_object_id = $1
  )
  AND NOT EXISTS (
      SELECT 1
      FROM context69.task_items AS item
      WHERE item.input_storage_object_id = $1
  )
