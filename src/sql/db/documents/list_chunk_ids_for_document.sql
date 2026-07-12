SELECT id AS "id!"
FROM (
    SELECT id FROM context69.document_chunks WHERE document_id = $1
    UNION ALL
    SELECT id FROM context69.document_translation_chunks WHERE document_id = $1
) chunks
