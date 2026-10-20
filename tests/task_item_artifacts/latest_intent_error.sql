SELECT last_error
FROM context69.library_storage_object_cleanup
WHERE object_id = $1
ORDER BY id DESC
LIMIT 1
