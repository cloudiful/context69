UPDATE context69.metadata_index_definitions
SET data_type = $2, value_kind = $3, sortable = $4, status = 'building',
    processed_documents = 0, error_message = NULL, updated_at = now()
WHERE index_id = $1
