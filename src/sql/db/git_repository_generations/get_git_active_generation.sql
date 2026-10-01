SELECT
    ag.repository_key,
    ag.generation_key,
    ag.activated_at
FROM context69.git_repository_active_generations ag
JOIN context69.git_repository_sources s ON s.repository_key = ag.repository_key
WHERE s.group_id = $1
  AND ag.repository_key = $2
