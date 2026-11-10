-- One tombstone including the exact physical target recorded at deletion.
SELECT
    id, file_id, document_id, group_id,
    target_endpoint, target_collection, deleted_at, purged_at
FROM context69.library_deleted_entities
WHERE id = $1
