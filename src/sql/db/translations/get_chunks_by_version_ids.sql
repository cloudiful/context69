SELECT translation_id, id, chunk_index, chunk_text
FROM context69.document_translation_chunks
WHERE translation_id = ANY($1)
ORDER BY translation_id, chunk_index
