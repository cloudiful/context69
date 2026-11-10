-- Updates the editable target, profile, and settings of one repository owned by
-- $1.
--
-- $1 group_id, $2 repository_key, $3 target_ref, $4 pinned_commit_action,
-- $5 pinned_commit_value, $6 index_profile, $7 include_patterns text[],
-- $8 exclude_patterns text[], $9 interval_seconds, $10 webhook_enabled,
-- $11 enabled, $12 retention_count.
--
-- A target, pin, or profile edit marks the current snapshot stale and clears the
-- last failure code: the stored index no longer describes the requested source.
-- A changed include/exclude rule also marks it stale. Other settings keep the
-- current status.
WITH updated AS (
    UPDATE context69.git_repository_sources s
    SET target_ref = COALESCE($3, s.target_ref),
        pinned_commit_sha = CASE
            WHEN $4 = 'clear' THEN NULL
            WHEN $4 = 'set' THEN $5
            ELSE s.pinned_commit_sha
        END,
        index_profile = COALESCE($6, s.index_profile),
        include_patterns = $7,
        exclude_patterns = $8,
        interval_seconds = $9,
        webhook_enabled = $10,
        enabled = $11,
        retention_count = $12,
        index_status = CASE
            WHEN NOT $11 THEN 'disabled'
            WHEN $3 IS NOT NULL OR $6 IS NOT NULL OR $4 <> 'keep' THEN 'stale'
            WHEN s.index_status = 'disabled' THEN 'pending'
            WHEN s.include_patterns IS DISTINCT FROM $7
              OR s.exclude_patterns IS DISTINCT FROM $8
            THEN 'stale'
            ELSE s.index_status
        END,
        refresh_dirty = CASE WHEN $11 THEN s.refresh_dirty ELSE FALSE END,
        last_error_code = CASE
            WHEN NOT $11 OR $3 IS NOT NULL OR $6 IS NOT NULL OR $4 <> 'keep' THEN NULL
            ELSE s.last_error_code
        END,
        updated_at = now()
    WHERE s.group_id = $1
      AND s.repository_key = $2
    RETURNING
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
        s.updated_at
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
    updated.pinned_commit_sha,
    updated.indexed_commit_sha,
    updated.index_profile,
    updated.include_patterns,
    updated.exclude_patterns,
    updated.interval_seconds,
    updated.webhook_enabled,
    updated.enabled,
    updated.retention_count,
    updated.created_by_user_id,
    updated.config_revision,
    updated.refresh_dirty,
    updated.refresh_requested_at,
    updated.last_refresh_attempt_at,
    updated.force_rebuild_requested_at,
    updated.deletion_requested_at,
    updated.last_skip_summary,
    updated.indexed_pipeline_version,
    updated.indexed_config_fingerprint,
    updated.index_status,
    updated.last_error_code,
    updated.last_indexed_at,
    updated.checkpoint_updated_at,
    updated.created_at,
    updated.updated_at,
    g.group_key,
    g.full_path AS group_path,
    g.visibility AS group_visibility,
    ag.generation_key AS "active_generation_key?"
FROM updated
JOIN context69.groups g ON g.id = updated.group_id
LEFT JOIN context69.git_repository_active_generations ag
    ON ag.repository_key = updated.repository_key
