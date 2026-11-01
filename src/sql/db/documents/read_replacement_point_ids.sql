-- read_replacement_point_ids.sql (issue 1138, P15; source hash semantics P23)
--
-- The point ids of one affected document that a replacement switch retires.
--
-- $2 is the desired NEW source hash of the replacement. The document's own
-- chunk rows are always retired, and a stored translation chunk is retired
-- exactly when its version was translated from a different source hash: every
-- translation that does not match the new content becomes unhydratable and
-- leaves the search results. A translation of the new hash survives a
-- same-hash reindex.
--
-- $3 adds the translation set to the document's own chunks.
SELECT ids.id AS "id!"
FROM (
    SELECT id FROM context69.document_chunks WHERE document_id = $1
    UNION
    SELECT chunk.id
    FROM context69.document_translation_chunks AS chunk
    JOIN context69.document_translation_versions AS version ON version.id = chunk.translation_id
    WHERE chunk.document_id = $1 AND version.source_record_hash <> $2 AND $3
) AS ids
ORDER BY ids.id
