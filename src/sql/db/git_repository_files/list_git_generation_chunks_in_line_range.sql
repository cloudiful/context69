-- Lists the stored chunk text that covers one requested line window, in source
-- order.
--
-- Only the chunk table is read: the raw acquisition bytes live in
-- context69.git_generation_blobs and are never selected or joined here, so an
-- HTTP content read cannot pull a whole blob or a provider blob id. A chunk
-- matches when its own inclusive range overlaps the window, so a window that
-- starts or ends inside a chunk still finds it.
--
-- $1 group_id, $2 repository_key, $3 generation_key, $4 file_key,
-- $5 start_line, $6 end_line, $7 limit, $8 offset.
SELECT
    c.chunk_key,
    c.generation_key,
    c.file_key,
    c.chunk_index,
    c.start_line,
    c.end_line,
    c.chunk_text,
    c.created_at
FROM context69.git_generation_chunks c
JOIN context69.git_generation_files f
    ON f.generation_key = c.generation_key
   AND f.file_key = c.file_key
JOIN context69.git_repository_generations g ON g.generation_key = c.generation_key
JOIN context69.git_repository_sources s ON s.repository_key = g.repository_key
WHERE s.group_id = $1
  AND g.repository_key = $2
  AND c.generation_key = $3
  AND c.file_key = $4
  AND c.end_line >= $5
  AND c.start_line <= $6
ORDER BY c.chunk_index ASC
LIMIT $7 OFFSET $8
