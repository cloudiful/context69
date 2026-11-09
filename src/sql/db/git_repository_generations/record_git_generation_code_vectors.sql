-- Records the exact code destination and acknowledged point count of one
-- generation. The statement is scoped to a building generation of a repository
-- owned by $1 and returns the stored metadata, so a foreign, failed, ready, or
-- already completed generation can never gain code metadata through it.
--
-- $1 group_id, $2 repository_key, $3 generation_key, $4 fingerprint,
-- $5 dimensions, $6 collection_name, $7 point_count.
INSERT INTO context69.git_generation_code_vectors (
    generation_key,
    fingerprint,
    dimensions,
    collection_name,
    point_count
)
SELECT
    g.generation_key,
    $4,
    $5,
    $6,
    $7
FROM context69.git_repository_generations g
JOIN context69.git_repository_sources s
    ON s.repository_key = g.repository_key
WHERE s.group_id = $1
  AND g.repository_key = $2
  AND g.generation_key = $3
  AND g.status = 'building'
ON CONFLICT (generation_key) DO UPDATE
SET fingerprint = EXCLUDED.fingerprint,
    dimensions = EXCLUDED.dimensions,
    collection_name = EXCLUDED.collection_name,
    point_count = EXCLUDED.point_count
RETURNING
    generation_key,
    fingerprint,
    dimensions,
    collection_name,
    point_count,
    created_at
