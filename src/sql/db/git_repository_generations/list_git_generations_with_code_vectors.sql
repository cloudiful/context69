-- Every generation of one repository with its optional code vector destination.
--
-- Used when a whole source is deleted: every recorded destination must be
-- cleared before the source row and its generation rows disappear. A lexical
-- generation has no vector row, so its code columns are null.
--
-- $1 group_id, $2 repository_key.
SELECT
    g.generation_key,
    g.status,
    g.cleanup_pending,
    v.fingerprint AS "fingerprint?",
    v.endpoint_identity AS "endpoint_identity?",
    v.dimensions AS "dimensions?",
    v.collection_name AS "collection_name?",
    v.point_count AS "point_count?",
    v.created_at AS "created_at?"
FROM context69.git_repository_generations g
JOIN context69.git_repository_sources s
    ON s.repository_key = g.repository_key
LEFT JOIN context69.git_generation_code_vectors v
    ON v.generation_key = g.generation_key
WHERE s.group_id = $1
  AND g.repository_key = $2
ORDER BY g.generation_number DESC
