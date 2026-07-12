SELECT c.id
FROM context69.document_translation_chunks c
JOIN context69.document_translation_versions v ON v.id = c.translation_id
JOIN context69.documents d ON d.id = v.document_id
WHERE v.document_id = $1 AND v.target_locale = $2
  AND v.source_record_hash = d.record_hash
