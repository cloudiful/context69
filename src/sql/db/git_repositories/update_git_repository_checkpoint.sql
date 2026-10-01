-- The checkpoint advances only for a repository owned by $1; the join also
-- re-reads the authoritative group row and active generation pointer.
WITH updated AS (
    UPDATE context69.git_repository_sources
    SET target_commit_sha = COALESCE($3, target_commit_sha),
        indexed_commit_sha = COALESCE($4, indexed_commit_sha),
        index_status = $5,
        checkpoint_updated_at = now(),
        last_indexed_at = CASE WHEN $4 IS NULL THEN last_indexed_at ELSE now() END,
        updated_at = now()
    WHERE group_id = $1
      AND repository_key = $2
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
    updated.group_id,
    updated.repository_key,
    updated.connection_key,
    updated.provider_kind,
    updated.canonical_url,
    updated.repository_owner,
    updated.repository_name,
    updated.default_branch,
    updated.target_ref,
    updated.target_commit_sha,
    updated.indexed_commit_sha,
    updated.index_profile,
    updated.refresh_policy,
    updated.index_status,
    updated.last_indexed_at,
    updated.checkpoint_updated_at,
    updated.created_at,
    updated.updated_at,
    g.group_key,
    g.full_path AS group_path,
    g.visibility AS group_visibility,
    ag.generation_key AS active_generation_key
FROM updated
JOIN context69.groups g ON g.id = updated.group_id
LEFT JOIN context69.git_repository_active_generations ag
    ON ag.repository_key = updated.repository_key
