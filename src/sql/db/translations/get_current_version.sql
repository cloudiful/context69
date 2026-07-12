SELECT v.id, v.target_locale, v.translated_title, v.translated_summary
FROM context69.document_translation_versions v
JOIN context69.documents d ON d.id = v.document_id
WHERE v.document_id = $1 AND v.target_locale = $2
  AND v.source_record_hash = d.record_hash
