SELECT group_id, sha256, object_key, storage_backend
FROM context69.library_storage_object_cleanup
WHERE object_id = $1 AND completed_at IS NULL
