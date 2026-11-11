-- Removes one non-active generation after its code vectors were deleted.
--
-- The active generation is refused even if a caller names it. Files, chunks,
-- and code-vector metadata cascade with the generation row.
--
-- $1 group_id, $2 repository_key, $3 generation_key.
DELETE FROM context69.git_repository_generations g
USING context69.git_repository_sources s
WHERE s.group_id = $1
  AND s.repository_key = $2
  AND g.repository_key = $2
  AND g.generation_key = $3
  AND NOT EXISTS (
      SELECT 1
      FROM context69.git_repository_active_generations ag
      WHERE ag.repository_key = g.repository_key
        AND ag.generation_key = g.generation_key
  )
RETURNING g.generation_key
