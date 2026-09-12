UPDATE context69.library_files
SET storage_object_id = NULL,
    source_released_at = now(),
    updated_at = now()
WHERE id = $1
  AND source_released_at IS NULL
RETURNING id
