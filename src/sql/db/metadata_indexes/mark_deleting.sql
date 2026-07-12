UPDATE context69.metadata_index_definitions
SET status = 'deleting', error_message = NULL, updated_at = now()
WHERE index_id = $1
