SELECT DISTINCT ON (v.document_id)
    v.document_id,
    v.id,
    v.target_locale,
    v.translated_title,
    v.translated_summary
FROM context69.document_translation_versions v
JOIN context69.documents d
  ON d.id = v.document_id
 AND d.record_hash = v.source_record_hash
WHERE v.document_id = ANY($1)
  AND v.target_locale = $2
ORDER BY v.document_id, v.created_at DESC, v.id DESC
