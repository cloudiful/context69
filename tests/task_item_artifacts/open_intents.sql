SELECT count(*) AS count
FROM context69.library_storage_object_cleanup
WHERE object_id = $1 AND completed_at IS NULL
