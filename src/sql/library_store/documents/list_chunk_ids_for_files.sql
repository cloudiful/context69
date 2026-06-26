SELECT c.id
FROM context69.document_chunks c
INNER JOIN context69.library_file_documents lfd ON lfd.document_id = c.document_id
WHERE lfd.file_id = ANY($1)
ORDER BY c.document_id, c.chunk_index
