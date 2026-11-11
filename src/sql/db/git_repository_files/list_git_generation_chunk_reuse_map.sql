-- Maps every chunk of a building generation that reuses baseline content to
-- the baseline chunk it can copy a vector from.
--
-- $1 group_id, $2 repository_key, $3 target_generation_key,
-- $4 baseline_generation_key, $5 limit, $6 offset.
--
-- A pair is reusable only when the target and baseline manifests hold the same
-- path with the same provider blob id and the same chunk index. A changed file
-- never matches, so its chunks are embedded rather than copied. The target
-- generation must still be building and the baseline must be a completed
-- generation of the same repository and group.
SELECT
    baseline_chunk.chunk_key AS source_chunk_key,
    target_chunk.chunk_key AS target_chunk_key,
    target_file.file_key AS target_file_key,
    target_file.path,
    target_file.language,
    target_chunk.chunk_index,
    target_chunk.start_line,
    target_chunk.end_line,
    target_chunk.chunk_text
FROM context69.git_repository_generations target
JOIN context69.git_repository_sources s ON s.repository_key = target.repository_key
JOIN context69.git_repository_generations baseline
    ON baseline.repository_key = target.repository_key
   AND baseline.generation_key = $4
   AND baseline.status IN ('ready', 'superseded')
JOIN context69.git_generation_files target_file
    ON target_file.generation_key = target.generation_key
JOIN context69.git_generation_files baseline_file
    ON baseline_file.generation_key = baseline.generation_key
   AND baseline_file.path = target_file.path
   AND baseline_file.provider_blob_sha = target_file.provider_blob_sha
JOIN context69.git_generation_chunks target_chunk
    ON target_chunk.generation_key = target.generation_key
   AND target_chunk.file_key = target_file.file_key
JOIN context69.git_generation_chunks baseline_chunk
    ON baseline_chunk.generation_key = baseline.generation_key
   AND baseline_chunk.file_key = baseline_file.file_key
   AND baseline_chunk.chunk_index = target_chunk.chunk_index
WHERE s.group_id = $1
  AND target.repository_key = $2
  AND target.generation_key = $3
  AND target.status = 'building'
ORDER BY target_file.path ASC, target_chunk.chunk_index ASC
LIMIT $5 OFFSET $6
