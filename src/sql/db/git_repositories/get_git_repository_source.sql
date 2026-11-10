-- Group visibility and path come from the authoritative groups row, never from
-- a stored snapshot; the active generation pointer is optional.
SELECT
    s.group_id,
    s.repository_key,
    s.connection_key,
    s.provider_kind,
    s.canonical_url,
    s.repository_owner,
    s.repository_name,
    s.default_branch,
    s.target_ref,
    s.target_commit_sha,
    s.pinned_commit_sha,
    s.indexed_commit_sha,
    s.index_profile,
    s.include_patterns,
    s.exclude_patterns,
    s.interval_seconds,
    s.webhook_enabled,
    s.enabled,
    s.retention_count,
    s.created_by_user_id,
    s.config_revision,
    s.refresh_dirty,
    s.refresh_requested_at,
    s.last_refresh_attempt_at,
    s.force_rebuild_requested_at,
    s.deletion_requested_at,
    s.last_skip_summary,
    s.indexed_pipeline_version,
    s.indexed_config_fingerprint,
    s.index_status,
    s.last_error_code,
    s.last_indexed_at,
    s.checkpoint_updated_at,
    s.created_at,
    s.updated_at,
    g.group_key,
    g.full_path AS group_path,
    g.visibility AS group_visibility,
    ag.generation_key AS "active_generation_key?"
FROM context69.git_repository_sources s
JOIN context69.groups g ON g.id = s.group_id
LEFT JOIN context69.git_repository_active_generations ag
    ON ag.repository_key = s.repository_key
WHERE s.group_id = $1
  AND s.repository_key = $2
