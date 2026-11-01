-- read_stale_translation_point_ids.sql (issue 1138, P23)
--
-- The translation point ids of one document whose stored translation version
-- does not match the desired NEW source hash.
--
-- The document's own chunks are not part of this set: a metadata publication
-- keeps them live and rewrites their payload in place.
--
-- An id that a registry entry already excludes is not reported again. The
-- bounded admission refuses an id set that overlaps an existing entry, and a
-- translation that an earlier publication already tombstoned must not fail
-- the next metadata publication that invalidates it once more.
SELECT chunk.id AS "id!"
FROM context69.document_translation_chunks AS chunk
JOIN context69.document_translation_versions AS version ON version.id = chunk.translation_id
WHERE chunk.document_id = $1
  AND version.source_record_hash <> $2
  AND NOT EXISTS (
      SELECT 1
      FROM context69.search_registry_entries AS entry
      WHERE entry.staged_ids && ARRAY[chunk.id]::uuid[]
         OR entry.retired_ids && ARRAY[chunk.id]::uuid[]
  )
ORDER BY chunk.id
