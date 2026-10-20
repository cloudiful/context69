UPDATE context69.library_storage_object_cleanup
SET next_attempt_at = now()
WHERE object_id = $1 AND completed_at IS NULL
