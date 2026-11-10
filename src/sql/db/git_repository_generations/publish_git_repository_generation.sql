-- Publishes a building generation and advances the source checkpoint in one
-- atomic statement (issue #28 phase P6).
--
-- The source row is locked first and must still match the captured
-- configuration revision, observed target, enabled state, and non-deleting
-- state. Only then are the generation completed, the previous active generation
-- superseded, the active pointer moved, and the source checkpoint/status/identity
-- updated. No separate checkpoint statement can split that publication.
--
-- A force demand is consumed only when it is not newer than the demand this run
-- observed; a request arriving during the run therefore survives.
--
-- $1 group_id, $2 repository_key, $3 generation_key,
-- $4 expected_config_revision, $5 observed_target_commit_sha,
-- $6 indexed_pipeline_version, $7 indexed_config_fingerprint,
-- $8 last_skip_summary, $9 file_count, $10 excluded_file_count,
-- $11 total_bytes, $12 observed_force_rebuild_requested_at.
WITH expected_source AS (
    SELECT s.repository_key
    FROM context69.git_repository_sources s
    WHERE s.group_id = $1
      AND s.repository_key = $2
      AND s.config_revision = $4
      AND s.target_commit_sha IS NOT DISTINCT FROM $5
      AND s.enabled
      AND s.deletion_requested_at IS NULL
    FOR UPDATE
),
completed AS (
    UPDATE context69.git_repository_generations g
    SET status = 'ready',
        file_count = $9,
        excluded_file_count = $10,
        total_bytes = $11,
        completed_at = now(),
        error_code = NULL,
        updated_at = now()
    FROM expected_source s
    WHERE g.repository_key = s.repository_key
      AND g.generation_key = $3
      AND g.status = 'building'
    RETURNING g.repository_key, g.generation_key
),
superseded AS (
    UPDATE context69.git_repository_generations g
    SET status = 'superseded',
        completed_at = COALESCE(g.completed_at, now()),
        updated_at = now()
    FROM context69.git_repository_active_generations ag
    WHERE ag.repository_key = (SELECT repository_key FROM completed)
      AND ag.generation_key <> (SELECT generation_key FROM completed)
      AND g.generation_key = ag.generation_key
      AND g.status = 'ready'
    RETURNING g.generation_key
),
source_updated AS (
    UPDATE context69.git_repository_sources s
    SET target_commit_sha = $5,
        indexed_commit_sha = $5,
        index_status = 'ready',
        indexed_pipeline_version = $6,
        indexed_config_fingerprint = $7,
        last_skip_summary = $8,
        last_error_code = NULL,
        last_indexed_at = now(),
        checkpoint_updated_at = now(),
        force_rebuild_requested_at = CASE
            WHEN $12::timestamptz IS NOT NULL
             AND s.force_rebuild_requested_at IS NOT NULL
             AND s.force_rebuild_requested_at <= $12
            THEN NULL
            ELSE s.force_rebuild_requested_at
        END,
        updated_at = now()
    FROM expected_source e, completed c
    WHERE s.repository_key = e.repository_key
      AND s.config_revision = $4
      AND s.target_commit_sha IS NOT DISTINCT FROM $5
      AND s.enabled
      AND s.deletion_requested_at IS NULL
    RETURNING s.repository_key
),
activated AS (
    INSERT INTO context69.git_repository_active_generations (repository_key, generation_key)
    SELECT completed.repository_key, completed.generation_key
    FROM completed
    JOIN source_updated ON source_updated.repository_key = completed.repository_key
    ON CONFLICT (repository_key) DO UPDATE
    SET generation_key = EXCLUDED.generation_key,
        activated_at = now()
    RETURNING repository_key, generation_key, activated_at
)
SELECT repository_key, generation_key, activated_at
FROM activated
