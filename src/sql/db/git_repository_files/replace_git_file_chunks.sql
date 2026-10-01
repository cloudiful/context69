-- Replaces the whole chunk list of one file of a building generation, in a
-- single statement: the previous chunks of that file leave and the new ones
-- land together, so a file is never readable with a mix of two revisions.
--
-- $1 group_id, $2 repository_key, $3 generation_key, $4 file_key,
-- $5 chunk_index int[], $6 start_line int[], $7 end_line int[], $8 text text[].
--
-- The file must be one of $3's own files, and that generation must still be
-- building; otherwise the statement reports an empty scope and writes nothing.
-- Chunk text is stored verbatim and anchored to inclusive 1-based lines, so
-- whitespace, line endings, and punctuation survive the round trip.
WITH scoped_file AS (
    SELECT f.file_key, f.generation_key
    FROM context69.git_generation_files f
    JOIN context69.git_repository_generations g ON g.generation_key = f.generation_key
    JOIN context69.git_repository_sources s ON s.repository_key = g.repository_key
    WHERE s.group_id = $1
      AND g.repository_key = $2
      AND f.generation_key = $3
      AND f.file_key = $4
      AND g.status = 'building'
),
incoming_chunks AS (
    SELECT
        scoped_file.generation_key,
        scoped_file.file_key,
        chunk.chunk_index,
        chunk.start_line,
        chunk.end_line,
        chunk.chunk_text
    FROM scoped_file
    CROSS JOIN LATERAL unnest($5::int[], $6::int[], $7::int[], $8::text[])
        AS chunk(chunk_index, start_line, end_line, chunk_text)
),
retired_chunks AS (
    DELETE FROM context69.git_generation_chunks c
    USING scoped_file
    WHERE c.file_key = scoped_file.file_key
    RETURNING 1
),
stored_chunks AS (
    INSERT INTO context69.git_generation_chunks (
        generation_key,
        file_key,
        chunk_index,
        start_line,
        end_line,
        chunk_text
    )
    SELECT
        generation_key,
        file_key,
        chunk_index,
        start_line,
        end_line,
        chunk_text
    FROM incoming_chunks
    RETURNING 1
)
SELECT
    (SELECT count(*) FROM scoped_file) AS "scoped_file_count!",
    (SELECT count(*) FROM retired_chunks) AS "retired_chunk_count!",
    (SELECT count(*) FROM stored_chunks) AS "stored_chunk_count!"
