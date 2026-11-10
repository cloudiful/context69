-- Inserts additional manifest entries into one building generation without
-- retiring entries another step already seeded.
--
-- $1 group_id, $2 repository_key, $3 generation_key, $4 path text[],
-- $5 language text[], $6 provider_blob_sha text[], $7 content bytea[],
-- $8 line_count bigint[].
--
-- The incremental pipeline first copies unchanged content from the baseline
-- generation and then writes only changed/new files here. A complete-manifest
-- replace would retire the seeded entries, so this statement is insert-only:
-- every entry the arrays describe lands together with the bytes it needs, and
-- nothing else in the generation is touched.
WITH scoped_generation AS (
    SELECT g.generation_key
    FROM context69.git_repository_generations g
    JOIN context69.git_repository_sources s ON s.repository_key = g.repository_key
    WHERE s.group_id = $1
      AND g.repository_key = $2
      AND g.generation_key = $3
      AND g.status = 'building'
),
incoming_files AS (
    SELECT
        scoped_generation.generation_key,
        incoming.path,
        incoming.language,
        incoming.provider_blob_sha,
        incoming.content,
        incoming.line_count
    FROM scoped_generation
    CROSS JOIN LATERAL unnest(
        $4::text[], $5::text[], $6::text[], $7::bytea[], $8::bigint[]
    ) AS incoming(path, language, provider_blob_sha, content, line_count)
),
incoming_blobs AS (
    SELECT DISTINCT ON (provider_blob_sha)
        generation_key,
        provider_blob_sha,
        content
    FROM incoming_files
    ORDER BY provider_blob_sha, octet_length(content)
),
stored_blobs AS (
    INSERT INTO context69.git_generation_blobs (
        generation_key,
        provider_blob_sha,
        byte_count,
        content
    )
    SELECT
        generation_key,
        provider_blob_sha,
        octet_length(content),
        content
    FROM incoming_blobs
    ON CONFLICT (generation_key, provider_blob_sha) DO NOTHING
    RETURNING 1
),
stored_files AS (
    INSERT INTO context69.git_generation_files (
        generation_key,
        repository_key,
        provider_blob_sha,
        path,
        language,
        byte_count,
        line_count
    )
    SELECT
        incoming.generation_key,
        $2,
        incoming.provider_blob_sha,
        incoming.path,
        incoming.language,
        octet_length(incoming.content),
        incoming.line_count
    FROM incoming_files incoming
    RETURNING 1
)
SELECT
    (SELECT count(*) FROM scoped_generation) AS "scoped_generation_count!",
    (SELECT count(*) FROM stored_files) AS "stored_file_count!"
