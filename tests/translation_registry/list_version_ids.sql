-- The translation version ids of a document and target locale.
SELECT id
FROM context69.document_translation_versions
WHERE document_id = $1 AND target_locale = $2
ORDER BY id
