WITH updated_document AS (
    UPDATE context69.documents
    SET external_id = $2,
        source_uri = $3,
        published_at = $4,
        updated_at_source = $5,
        metadata_json = $6,
        record_hash = $7,
        updated_at = now()
    WHERE id = $1
    RETURNING id
)
UPDATE context69.document_chunks
SET record_hash = $7
WHERE document_id IN (SELECT id FROM updated_document)
