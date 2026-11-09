-- Hydrates canonical stored chunks by chunk key from one repository's active
-- ready generation.
--
-- $1 group_id (owning group), $2 repository_key, $3 captured generation_key,
-- $4 chunk_keys uuid[], $5 path prefix LIKE pattern or NULL, $6 language or
-- NULL, $7 visible_group_ids bigint[].
--
-- The statement is the only authority for text and provenance: a candidate key
-- answers only when it is stored under the captured generation, that generation
-- is still the active ready one, the repository belongs to $1, and the owning
-- group's current visibility admits the caller. A forged, stale, superseded, or
-- cross-scope remote key therefore matches no row instead of leaking content.
WITH repository_scope AS (
    SELECT s.repository_key, s.group_id, g.visibility AS group_visibility
    FROM context69.git_repository_sources s
    JOIN context69.groups g ON g.id = s.group_id
    WHERE s.group_id = $1
      AND s.repository_key = $2
),
active_generation AS (
    SELECT gen.generation_key, gen.generation_number, gen.ref_name, gen.commit_sha
    FROM context69.git_repository_active_generations ag
    JOIN context69.git_repository_generations gen ON gen.generation_key = ag.generation_key
    JOIN repository_scope r ON r.repository_key = ag.repository_key
    WHERE ag.repository_key = r.repository_key
      AND gen.generation_key = $3
      AND gen.status = 'ready'
)
SELECT
    r.repository_key AS "repository_key!",
    gen.generation_key AS "generation_key!",
    gen.generation_number AS "generation_number!",
    gen.ref_name AS "ref_name!",
    gen.commit_sha AS "commit_sha!",
    r.group_visibility AS "group_visibility!",
    f.file_key AS "file_key!",
    f.path AS "path!",
    f.language AS "language!",
    c.chunk_key AS "chunk_key!",
    c.chunk_index AS "chunk_index!",
    c.start_line AS "start_line!",
    c.end_line AS "end_line!",
    c.chunk_text AS "chunk_text!"
FROM context69.git_generation_chunks c
JOIN context69.git_generation_files f
    ON f.generation_key = c.generation_key
   AND f.file_key = c.file_key
JOIN active_generation gen ON gen.generation_key = c.generation_key
JOIN repository_scope r ON TRUE
WHERE c.chunk_key = ANY($4::uuid[])
  AND ($5::text IS NULL OR f.path LIKE $5 ESCAPE '\')
  AND ($6::text IS NULL OR f.language = $6)
  AND (r.group_visibility = 'public' OR r.group_id = ANY($7::bigint[]))
ORDER BY f.path ASC, c.chunk_index ASC, c.chunk_key ASC
