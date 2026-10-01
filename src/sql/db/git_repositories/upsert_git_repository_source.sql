-- Per-group repository/ref identity: the conflict target is the owning group
-- plus the repository ref, so two groups register the same ref independently.
WITH upserted AS (
    INSERT INTO context69.git_repository_sources (
        group_id,
        connection_key,
        provider_kind,
        canonical_url,
        repository_owner,
        repository_name,
        default_branch,
        target_ref,
        target_commit_sha,
        index_profile,
        refresh_policy
    )
    VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
    ON CONFLICT (group_id, canonical_url, target_ref) DO UPDATE
    SET connection_key = EXCLUDED.connection_key,
        provider_kind = EXCLUDED.provider_kind,
        repository_owner = EXCLUDED.repository_owner,
        repository_name = EXCLUDED.repository_name,
        default_branch = EXCLUDED.default_branch,
        target_commit_sha = COALESCE(
            EXCLUDED.target_commit_sha,
            context69.git_repository_sources.target_commit_sha
        ),
        index_profile = EXCLUDED.index_profile,
        refresh_policy = EXCLUDED.refresh_policy,
        updated_at = now()
    RETURNING
        group_id,
        repository_key,
        connection_key,
        provider_kind,
        canonical_url,
        repository_owner,
        repository_name,
        default_branch,
        target_ref,
        target_commit_sha,
        indexed_commit_sha,
        index_profile,
        refresh_policy,
        index_status,
        last_indexed_at,
        checkpoint_updated_at,
        created_at,
        updated_at
)
SELECT
    upserted.group_id,
    upserted.repository_key,
    upserted.connection_key,
    upserted.provider_kind,
    upserted.canonical_url,
    upserted.repository_owner,
    upserted.repository_name,
    upserted.default_branch,
    upserted.target_ref,
    upserted.target_commit_sha,
    upserted.indexed_commit_sha,
    upserted.index_profile,
    upserted.refresh_policy,
    upserted.index_status,
    upserted.last_indexed_at,
    upserted.checkpoint_updated_at,
    upserted.created_at,
    upserted.updated_at,
    g.group_key,
    g.full_path AS group_path,
    g.visibility AS group_visibility,
    ag.generation_key AS active_generation_key
FROM upserted
JOIN context69.groups g ON g.id = upserted.group_id
LEFT JOIN context69.git_repository_active_generations ag
    ON ag.repository_key = upserted.repository_key
