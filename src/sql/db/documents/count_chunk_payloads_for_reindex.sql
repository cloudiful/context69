SELECT COUNT(*) AS "count!"
FROM context69.document_chunks c
INNER JOIN context69.documents d ON d.id = c.document_id
