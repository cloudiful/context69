DELETE FROM context69.document_translation_versions
WHERE document_id = $1 AND target_locale = $2 AND source_record_hash = $3
