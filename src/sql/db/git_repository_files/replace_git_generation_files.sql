-- Replaces the whole path manifest of one building generation together with the
-- raw bytes it needs, in a single statement.
--
-- $1 group_id, $2 repository_key, $3 generation_key, $4 path text[],
-- $5 language text[], $6 provider_blob_sha text[], $7 content bytea[],
-- $8 line_count bigint[].
--
-- The manifest is complete rather than incremental: a stored file the arrays do
-- not describe is retired, its chunks leave with it, and bytes no described file
-- needs are removed with it. A stored file the arrays describe identically (same
-- path, bytes, language, and line count) keeps its key and its chunks, so
-- re-sending unchanged content costs nothing and the deduplicated bytes are
-- stored once per (generation, blob id).
--
-- Every write is scoped to a building generation of a repository owned by $1: a
-- repository of another group, an unknown generation, or a generation that is no
-- longer building matches no row, so the statement reports an empty scope and
-- writes nothing.
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
-- Stored files the incoming manifest describes exactly: same path, bytes,
-- language, and line count. They are left untouched.
retained_files AS (
    SELECT f.file_key, f.path
    FROM context69.git_generation_files f
    JOIN scoped_generation g ON g.generation_key = f.generation_key
    WHERE EXISTS (
        SELECT 1
        FROM incoming_files incoming
        WHERE incoming.path = f.path
          AND incoming.provider_blob_sha = f.provider_blob_sha
          AND incoming.language = f.language
          AND incoming.line_count = f.line_count
    )
),
-- One row per blob id, so identical bytes are stored once per generation.
incoming_blobs AS (
    SELECT DISTINCT ON (provider_blob_sha)
        generation_key,
        provider_blob_sha,
        content
    FROM incoming_files
    ORDER BY provider_blob_sha, octet_length(content)
),
-- A manifest that cannot describe one snapshot is refused whole: two different
-- byte strings for one content-addressed id is a provider inconsistency rather
-- than deduplication, and the same path twice would leave the manifest
-- ambiguous. Every write below is skipped while a conflict is reported.
manifest_conflicts AS (
    SELECT 'blob_bytes'::TEXT AS kind
    FROM incoming_files
    GROUP BY provider_blob_sha
    HAVING count(DISTINCT content) > 1
    UNION ALL
    SELECT 'duplicate_path'::TEXT
    FROM incoming_files
    GROUP BY path
    HAVING count(*) > 1
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
    WHERE NOT EXISTS (SELECT 1 FROM manifest_conflicts)
    ON CONFLICT (generation_key, provider_blob_sha) DO NOTHING
    RETURNING 1
),
-- A file the manifest no longer describes, or describes with different content
-- or classification, is retired; its chunks leave with it, so text of a
-- superseded revision can never outlive the bytes it was cut from.
retired_files AS (
    DELETE FROM context69.git_generation_files f
    USING scoped_generation g
    WHERE f.generation_key = g.generation_key
      AND NOT EXISTS (SELECT 1 FROM manifest_conflicts)
      AND NOT EXISTS (
          SELECT 1
          FROM retained_files kept
          WHERE kept.file_key = f.file_key
      )
    RETURNING f.generation_key, f.provider_blob_sha
),
-- Bytes the manifest no longer references leave with the files that referenced
-- them. A blob the manifest still needs is never deleted, so the file rows
-- inserted below always resolve their bytes.
retired_blobs AS (
    DELETE FROM context69.git_generation_blobs b
    USING scoped_generation g, retired_files removed
    WHERE b.generation_key = g.generation_key
      AND b.generation_key = removed.generation_key
      AND b.provider_blob_sha = removed.provider_blob_sha
      AND NOT EXISTS (SELECT 1 FROM manifest_conflicts)
      AND NOT EXISTS (
          SELECT 1
          FROM incoming_files incoming
          WHERE incoming.provider_blob_sha = b.provider_blob_sha
      )
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
    WHERE NOT EXISTS (SELECT 1 FROM manifest_conflicts)
      AND NOT EXISTS (
          SELECT 1
          FROM retained_files kept
          WHERE kept.path = incoming.path
      )
    RETURNING 1
)
SELECT
    (SELECT count(*) FROM scoped_generation) AS "scoped_generation_count!",
    (SELECT count(*) FROM manifest_conflicts WHERE kind = 'blob_bytes')
        AS "conflicting_blob_count!",
    (SELECT count(*) FROM manifest_conflicts WHERE kind = 'duplicate_path')
        AS "duplicate_path_count!",
    (SELECT count(*) FROM stored_blobs) AS "stored_blob_count!",
    (SELECT count(*) FROM retired_files) AS "retired_file_count!",
    (SELECT count(*) FROM retired_blobs) AS "retired_blob_count!",
    (SELECT count(*) FROM stored_files) AS "stored_file_count!"
