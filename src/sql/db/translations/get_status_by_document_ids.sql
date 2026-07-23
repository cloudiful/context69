SELECT DISTINCT ON (document_id)
    document_id,
    status,
    COALESCE(detected_source_locale, requested_source_locale) AS source_locale
FROM context69.document_translation_jobs
WHERE document_id = ANY($1)
  AND target_locale = $2
ORDER BY document_id, created_at DESC, id DESC
