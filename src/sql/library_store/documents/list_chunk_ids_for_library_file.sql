SELECT id AS "id!"
FROM (
    SELECT chunk.id
    FROM context69.document_chunks chunk
    INNER JOIN context69.documents document ON document.id = chunk.document_id
    WHERE document.metadata_json->>'library_file_id' = $1
    UNION ALL
    SELECT chunk.id
    FROM context69.document_translation_chunks chunk
    INNER JOIN context69.documents document ON document.id = chunk.document_id
    WHERE document.metadata_json->>'library_file_id' = $1
) chunks
