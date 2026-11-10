-- Remove the live original chunks of every staged document, including the
-- documents whose fresh body produced zero chunks.
DELETE FROM context69.document_chunks
WHERE document_id IN (
    SELECT document_id
    FROM context69.document_rebuild_staging_documents
    WHERE target_collection = $1
)
