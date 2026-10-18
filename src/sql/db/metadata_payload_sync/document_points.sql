SELECT d.record_hash,
       COALESCE(
           (SELECT array_agg(chunk.id ORDER BY chunk.id)
            FROM context69.document_chunks chunk
            WHERE chunk.document_id = d.id),
           ARRAY[]::uuid[]
       ) AS originals,
       COALESCE(
           (SELECT array_agg(translation.id ORDER BY translation.id)
            FROM context69.document_translation_chunks translation
            WHERE translation.document_id = d.id),
           ARRAY[]::uuid[]
       ) AS translations
FROM context69.documents d
WHERE d.id = $1
