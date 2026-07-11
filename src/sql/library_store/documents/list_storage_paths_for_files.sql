SELECT id, storage_rel_path, storage_object_id
FROM context69.library_files
WHERE id = ANY($1)
ORDER BY filename, id
