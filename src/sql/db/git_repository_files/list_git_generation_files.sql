-- Lists one page of a generation's path manifest, ordered by path so paging is
-- stable while a snapshot is being built.
--
-- $1 group_id, $2 repository_key, $3 generation_key, $4 limit, $5 offset.
SELECT
    f.file_key,
    f.generation_key,
    f.repository_key,
    f.provider_blob_sha,
    f.path,
    f.language,
    f.byte_count,
    f.line_count,
    f.created_at
FROM context69.git_generation_files f
JOIN context69.git_repository_generations g ON g.generation_key = f.generation_key
JOIN context69.git_repository_sources s ON s.repository_key = g.repository_key
WHERE s.group_id = $1
  AND g.repository_key = $2
  AND f.generation_key = $3
ORDER BY f.path ASC
LIMIT $4 OFFSET $5
