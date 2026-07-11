UPDATE context69.library_files
SET storage_object_id = $2,
    storage_rel_path = $3,
    updated_at = now()
WHERE id = $1
