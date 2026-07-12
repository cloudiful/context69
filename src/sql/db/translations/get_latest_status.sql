SELECT status, COALESCE(detected_source_locale, requested_source_locale) AS source_locale
FROM context69.document_translation_jobs
WHERE document_id = $1 AND target_locale = $2
ORDER BY created_at DESC, id DESC
LIMIT 1
