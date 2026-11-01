-- Every stored translation chunk id of a document.
--
-- $2 selects one target locale and a NULL selects every locale. The result
-- holds the ids of every stored version, the current one and the stale ones,
-- so a caller can reserve and retire the complete set a locale owns instead
-- of only the ids that match the current source hash.
SELECT c.id
FROM context69.document_translation_chunks c
JOIN context69.document_translation_versions v ON v.id = c.translation_id
WHERE v.document_id = $1
  AND ($2::text IS NULL OR v.target_locale = $2::text)
ORDER BY c.id
