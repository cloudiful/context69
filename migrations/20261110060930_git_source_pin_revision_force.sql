-- Separate the configured pin from the observed target and version source
-- configuration edits (issue #28 phase P6).
--
-- `target_commit_sha` remains the observed target a run resolved from the ref
-- (or the configured pin when one is set). `pinned_commit_sha` is the explicit
-- configured pin only, so a branch head resolved by an earlier run is never
-- reinterpreted as a fixed pin. Existing rows keep no inferred pin: guessing
-- that an observed target was configured would silently pin advancing branches.
--
-- `config_revision` is a monotonic fence for every edit that changes what a
-- snapshot means: target ref/branch, configured pin, index profile, selection
-- filters, enabled state, and connection. A run captures the revision it read
-- and may only publish while that revision is still current.
--
-- `force_rebuild_requested_at` records a force demand that arrived while
-- another index task was active. The completing run consumes only the demand it
-- observed, so a later request survives for the follow-up admission.

ALTER TABLE context69.git_repository_sources
    ADD COLUMN pinned_commit_sha TEXT,
    ADD COLUMN config_revision BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN force_rebuild_requested_at TIMESTAMPTZ;

CREATE OR REPLACE FUNCTION context69.bump_git_repository_source_config_revision()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    NEW.config_revision := OLD.config_revision + 1;
    RETURN NEW;
END;
$$;

CREATE TRIGGER trg_git_repository_sources_config_revision
    BEFORE UPDATE ON context69.git_repository_sources
    FOR EACH ROW
    WHEN (
        OLD.target_ref IS DISTINCT FROM NEW.target_ref
        OR OLD.default_branch IS DISTINCT FROM NEW.default_branch
        OR OLD.pinned_commit_sha IS DISTINCT FROM NEW.pinned_commit_sha
        OR OLD.index_profile IS DISTINCT FROM NEW.index_profile
        OR OLD.include_patterns IS DISTINCT FROM NEW.include_patterns
        OR OLD.exclude_patterns IS DISTINCT FROM NEW.exclude_patterns
        OR OLD.enabled IS DISTINCT FROM NEW.enabled
        OR OLD.connection_key IS DISTINCT FROM NEW.connection_key
    )
    EXECUTE FUNCTION context69.bump_git_repository_source_config_revision();
