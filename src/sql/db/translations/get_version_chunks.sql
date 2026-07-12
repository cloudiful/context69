SELECT id, chunk_index, chunk_text
FROM context69.document_translation_chunks
WHERE translation_id = $1
ORDER BY chunk_index
