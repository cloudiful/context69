UPDATE context69.metadata_index_definitions
SET status = 'failed', error_message = $2, updated_at = now()
WHERE index_id = $1
