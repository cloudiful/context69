-- The number of open physical-deletion intents of one storage object.
SELECT COUNT(*) AS "count!"
FROM context69.library_storage_object_cleanup
WHERE object_id = $1
