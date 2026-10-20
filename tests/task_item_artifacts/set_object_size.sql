UPDATE context69.library_storage_objects
SET size_bytes = $2,
    updated_at = now()
WHERE id = $1
