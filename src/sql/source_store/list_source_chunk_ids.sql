SELECT dc.id
FROM context69.document_chunks dc
JOIN context69.documents d ON d.id = dc.document_id
WHERE d.source_key = $1
ORDER BY dc.id
