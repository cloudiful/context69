-- Marks a still-building generation failed with a bounded error code. A
-- generation is never activated by this path, and an already completed
-- generation is left untouched.
UPDATE context69.git_repository_generations g
SET status = 'failed',
    error_code = $4,
    completed_at = now(),
    updated_at = now()
FROM context69.git_repository_sources s
WHERE s.group_id = $1
  AND s.repository_key = $2
  AND g.repository_key = s.repository_key
  AND g.generation_key = $3
  AND g.status = 'building'
RETURNING g.generation_key
