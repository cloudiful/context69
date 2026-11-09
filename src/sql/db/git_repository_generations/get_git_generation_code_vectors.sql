-- Reads the code vector metadata of one generation of a repository owned by $1.
-- A generation without recorded code vectors reads as no row, which is the
-- lexical-only outcome.
--
-- $1 group_id, $2 repository_key, $3 generation_key.
SELECT
    v.generation_key,
    v.fingerprint,
    v.dimensions,
    v.collection_name,
    v.point_count,
    v.created_at
FROM context69.git_generation_code_vectors v
JOIN context69.git_repository_generations g
    ON g.generation_key = v.generation_key
JOIN context69.git_repository_sources s
    ON s.repository_key = g.repository_key
WHERE s.group_id = $1
  AND g.repository_key = $2
  AND g.generation_key = $3
