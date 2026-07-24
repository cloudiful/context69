SELECT i.index_id, i.group_id, g.full_path AS group_path, i.source_key,
       i.field_path, i.data_type, i.value_kind, i.sortable, i.status,
       i.processed_documents, i.total_documents, i.error_message,
       i.created_at, i.updated_at
FROM context69.metadata_index_definitions i
JOIN context69.groups g ON g.id = i.group_id
WHERE i.group_id = $1 AND i.source_key = $2
ORDER BY i.field_path
LIMIT $3 OFFSET $4
