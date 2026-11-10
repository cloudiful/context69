-- Copies unchanged manifest entries and their chunks from a completed baseline
-- generation into a building generation, assigning fresh identities.
--
-- $1 group_id, $2 repository_key, $3 target_generation_key,
-- $4 baseline_generation_key, $5 path text[], $6 provider_blob_sha text[].
--
-- Each (path, blob id) pair must exist in the baseline generation with exactly
-- that content address. Bytes, file rows, and line-anchored chunks are copied
-- verbatim, so an unchanged file that a later vector pass reuses keeps exact
-- text and line provenance while the new snapshot owns independent rows.
WITH scoped AS (
    SELECT
        target.generation_key AS target_generation_key,
        baseline.generation_key AS baseline_generation_key,
        target.repository_key
    FROM context69.git_repository_generations target
    JOIN context69.git_repository_sources s ON s.repository_key = target.repository_key
    JOIN context69.git_repository_generations baseline
        ON baseline.repository_key = target.repository_key
    WHERE s.group_id = $1
      AND target.repository_key = $2
      AND target.generation_key = $3
      AND target.status = 'building'
      AND baseline.generation_key = $4
      AND baseline.status IN ('ready', 'superseded')
),
incoming AS (
    SELECT
        scoped.target_generation_key,
        scoped.baseline_generation_key,
        scoped.repository_key,
        entry.path,
        entry.provider_blob_sha
    FROM scoped
    CROSS JOIN LATERAL unnest($5::text[], $6::text[])
        AS entry(path, provider_blob_sha)
),
matched AS (
    SELECT
        incoming.target_generation_key,
        incoming.baseline_generation_key,
        incoming.repository_key,
        incoming.path,
        incoming.provider_blob_sha,
        baseline_file.file_key AS baseline_file_key,
        baseline_file.language,
        baseline_file.byte_count,
        baseline_file.line_count
    FROM incoming
    JOIN context69.git_generation_files baseline_file
        ON baseline_file.generation_key = incoming.baseline_generation_key
       AND baseline_file.path = incoming.path
       AND baseline_file.provider_blob_sha = incoming.provider_blob_sha
),
stored_blobs AS (
    INSERT INTO context69.git_generation_blobs (
        generation_key,
        provider_blob_sha,
        byte_count,
        content
    )
    SELECT DISTINCT
        matched.target_generation_key,
        matched.provider_blob_sha,
        baseline_blob.byte_count,
        baseline_blob.content
    FROM matched
    JOIN context69.git_generation_blobs baseline_blob
        ON baseline_blob.generation_key = matched.baseline_generation_key
       AND baseline_blob.provider_blob_sha = matched.provider_blob_sha
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
        matched.target_generation_key,
        matched.repository_key,
        matched.provider_blob_sha,
        matched.path,
        matched.language,
        matched.byte_count,
        matched.line_count
    FROM matched
    RETURNING file_key, path
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
        matched.target_generation_key,
        stored_files.file_key,
        baseline_chunk.chunk_index,
        baseline_chunk.start_line,
        baseline_chunk.end_line,
        baseline_chunk.chunk_text
    FROM matched
    JOIN stored_files ON stored_files.path = matched.path
    JOIN context69.git_generation_chunks baseline_chunk
        ON baseline_chunk.generation_key = matched.baseline_generation_key
       AND baseline_chunk.file_key = matched.baseline_file_key
    RETURNING 1
)
SELECT
    (SELECT count(*) FROM scoped) AS "scoped_generation_count!",
    (SELECT count(*) FROM stored_files) AS "stored_file_count!",
    (SELECT count(*) FROM stored_chunks) AS "stored_chunk_count!"
