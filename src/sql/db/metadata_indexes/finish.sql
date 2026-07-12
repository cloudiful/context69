UPDATE context69.metadata_index_definitions
SET status = 'ready', processed_documents = $2, total_documents = $2,
    error_message = NULL, updated_at = now()
WHERE index_id = $1
