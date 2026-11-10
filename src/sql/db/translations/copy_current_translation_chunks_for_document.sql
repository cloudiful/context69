-- Copy the current translated chunks of one document into rebuild staging.
--
-- Only the version whose source_record_hash equals the document's current
-- record_hash is current. Ids and texts are preserved exactly; translations are
-- never re-chunked by a rebuild.
INSERT INTO context69.document_translation_rebuild_staging (
    target_collection,
    translation_id,
    document_id,
    chunk_index,
    chunk_id,
    chunk_text,
    record_hash
)
SELECT $1, t.id, c.document_id, c.chunk_index, c.id, c.chunk_text, d.record_hash
FROM context69.document_translation_chunks c
INNER JOIN context69.document_translation_versions t ON t.id = c.translation_id
INNER JOIN context69.documents d ON d.id = c.document_id
WHERE c.document_id = $2
  AND t.source_record_hash = d.record_hash
ON CONFLICT (target_collection, translation_id, chunk_index) DO NOTHING
