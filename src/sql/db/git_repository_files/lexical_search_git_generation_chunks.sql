-- Bounded lexical search over the code chunks of one repository's active
-- generation.
--
-- $1 group_id (owning group), $2 repository_key, $3 phrase (lowercased query
-- text, used only for the exact path comparison), $4 phrase pattern (LIKE
-- pattern built from the same query with `\`, `%`, and `_` escaped), $5 terms
-- text[] (code-aware tokens, already pattern-escaped), $6 path prefix pattern
-- or NULL, $7 language or NULL, $8 visible_group_ids bigint[], $9 limit.
--
-- Only the activated generation is searched, and only while it is the ready one:
-- a building, failed, or superseded snapshot never answers, so results always
-- describe the snapshot a reader would get from the repository itself. The
-- group join reads the owner's current visibility on every call instead of a
-- stored copy, and a repository of another group matches no row.
--
-- The caller's text is matched whole and case-insensitively, and the token
-- filter keeps identifier characters (`_`, `.`, `:`, `/`, `-`, `$`) inside a
-- token, so `parse_ref` and `SafeRef::parse` are never shredded into prose
-- terms. Every LIKE pattern here is the escaped $4, in the path branch as well
-- as the chunk branch: a query of `100%` looks for a literal percent sign and a
-- query of only metacharacters cannot widen to every path. The raw $3 is used
-- only for `=` in the score and the match kind. Matching uses ILIKE, which is
-- correct on a sequential scan: the trigram index is only an accelerator, and
-- the same rows are returned without the extension. Callers bound the result
-- with $9.
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
      AND gen.status = 'ready'
),
matched AS (
    SELECT
        gen.generation_key,
        gen.generation_number,
        gen.ref_name,
        gen.commit_sha,
        r.repository_key,
        r.group_visibility,
        f.file_key,
        f.path,
        f.language,
        c.chunk_key,
        c.chunk_index,
        c.start_line,
        c.end_line,
        c.chunk_text
    FROM context69.git_generation_chunks c
    JOIN context69.git_generation_files f
        ON f.generation_key = c.generation_key
       AND f.file_key = c.file_key
    JOIN active_generation gen ON gen.generation_key = c.generation_key
    JOIN repository_scope r ON TRUE
    WHERE ($6::text IS NULL OR f.path LIKE $6 ESCAPE '\')
      AND ($7::text IS NULL OR f.language = $7)
      AND (r.group_visibility = 'public' OR r.group_id = ANY($8::bigint[]))
      AND (
          lower(c.chunk_text) LIKE $4 ESCAPE '\'
          OR lower(f.path) LIKE $4 ESCAPE '\'
          OR (
              cardinality($5::text[]) > 0
              AND NOT EXISTS (
                  SELECT 1
                  FROM unnest($5::text[]) AS term(value)
                  WHERE lower(c.chunk_text) NOT LIKE ('%' || term.value || '%') ESCAPE '\'
              )
          )
      )
),
scored AS (
    SELECT
        matched.*,
        -- The raw phrase is used only for equality; every LIKE pattern is the
        -- escaped one, so a `%`, `_`, or `\` in the query stays a literal
        -- character instead of widening the path branch.
        (CASE WHEN lower(matched.path) = $3 THEN 1.20
              WHEN lower(matched.path) LIKE $4 ESCAPE '\' THEN 0.90
              ELSE 0.00
          END
         + CASE WHEN lower(matched.chunk_text) LIKE $4 ESCAPE '\' THEN 0.82 ELSE 0.00 END
         + CASE WHEN cardinality($5::text[]) > 0 THEN 0.30 ELSE 0.00 END)::real AS score
    FROM matched
)
SELECT
    repository_key AS "repository_key!",
    generation_key AS "generation_key!",
    generation_number AS "generation_number!",
    ref_name AS "ref_name!",
    commit_sha AS "commit_sha!",
    group_visibility AS "group_visibility!",
    file_key AS "file_key!",
    path AS "path!",
    language AS "language!",
    chunk_key AS "chunk_key!",
    chunk_index AS "chunk_index!",
    start_line AS "start_line!",
    end_line AS "end_line!",
    chunk_text AS "chunk_text!",
    score AS "score!",
    CASE WHEN lower(path) = $3 THEN 'path_exact'
         WHEN lower(path) LIKE $4 ESCAPE '\' THEN 'path_phrase'
         WHEN lower(chunk_text) LIKE $4 ESCAPE '\' THEN 'chunk_phrase'
         ELSE 'chunk_terms'
    END AS "matched!"
FROM scored
ORDER BY "score!" DESC, path ASC, chunk_index ASC
LIMIT $9
