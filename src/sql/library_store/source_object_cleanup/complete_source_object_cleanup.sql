UPDATE context69.library_storage_object_cleanup
SET completed_at = now(),
    last_error = $2,
    updated_at = now()
WHERE id = $1
  AND completed_at IS NULL
RETURNING id
