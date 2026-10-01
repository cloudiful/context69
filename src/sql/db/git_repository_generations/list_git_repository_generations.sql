SELECT
    g.repository_key,
    g.generation_key,
    g.generation_number,
    g.ref_name,
    g.commit_sha,
    g.index_profile,
    g.status,
    g.file_count,
    g.excluded_file_count,
    g.total_bytes,
    g.error_code,
    g.started_at,
    g.completed_at,
    g.created_at,
    g.updated_at
FROM context69.git_repository_generations g
JOIN context69.git_repository_sources s ON s.repository_key = g.repository_key
WHERE s.group_id = $1
  AND g.repository_key = $2
ORDER BY g.generation_number DESC
