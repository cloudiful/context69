-- Completes a generation and makes it the repository's active generation in a
-- single statement, so the completion, the supersession of the previously
-- active generation, and the active-generation pointer move together: no window
-- exists where a generation is ready but not active, active but not ready, or
-- lost after a crash mid-statement.
--
-- $1 group_id, $2 repository_key, $3 generation_key,
-- $4 file_count, $5 excluded_file_count, $6 total_bytes.
WITH scoped_repository AS (
    SELECT s.repository_key
    FROM context69.git_repository_sources s
    WHERE s.group_id = $1
      AND s.repository_key = $2
),
completed AS (
    UPDATE context69.git_repository_generations g
    SET status = 'ready',
        file_count = $4,
        excluded_file_count = $5,
        total_bytes = $6,
        completed_at = now(),
        updated_at = now()
    FROM scoped_repository r
    WHERE g.repository_key = r.repository_key
      AND g.generation_key = $3
      AND g.status = 'building'
    RETURNING g.repository_key, g.generation_key
),
superseded AS (
    UPDATE context69.git_repository_generations g
    SET status = 'superseded',
        completed_at = COALESCE(g.completed_at, now()),
        updated_at = now()
    FROM context69.git_repository_active_generations ag
    WHERE ag.repository_key = (SELECT repository_key FROM completed)
      AND ag.generation_key <> (SELECT generation_key FROM completed)
      AND g.generation_key = ag.generation_key
      AND g.status = 'ready'
    RETURNING g.generation_key
)
INSERT INTO context69.git_repository_active_generations (repository_key, generation_key)
SELECT repository_key, generation_key
FROM completed
ON CONFLICT (repository_key) DO UPDATE
SET generation_key = EXCLUDED.generation_key,
    activated_at = now()
RETURNING
    repository_key,
    generation_key,
    activated_at
