-- Refresh demand, background refresh attempts, and generation retention
-- ownership for Git repository sources (issue #28 phase P4).
--
-- A webhook delivery or an in-flight manual request records a durable demand
-- marker on the source instead of relying on the request that happened to
-- create a task. The marker survives an active run, so a push during indexing
-- always leaves a follow-up demand rather than being silently dropped.
--
-- `last_refresh_attempt_at` bounds periodic retries: the scheduler selects a
-- source only after its interval has elapsed from the last attempt, so a
-- permanently failing source cannot hot-loop.
--
-- `created_by_user_id` names the manager whose registration created the source;
-- background refresh and authenticated webhook work are attributed to that
-- user. Existing rows keep NULL and the scheduler falls back to the group owner
-- or a current maintainer.
--
-- Generation cleanup ownership is explicit: a retired generation that could not
-- have its code vectors removed stays visible with `cleanup_pending` so a later
-- attempt can finish the removal before the metadata is deleted.

ALTER TABLE context69.git_repository_sources
    ADD COLUMN created_by_user_id BIGINT
        REFERENCES context69.users(id) ON DELETE SET NULL,
    ADD COLUMN refresh_dirty BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN refresh_requested_at TIMESTAMPTZ,
    ADD COLUMN last_refresh_attempt_at TIMESTAMPTZ,
    ADD COLUMN deletion_requested_at TIMESTAMPTZ;

ALTER TABLE context69.git_repository_generations
    ADD COLUMN cleanup_pending BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN cleanup_error TEXT,
    ADD COLUMN cleanup_attempted_at TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS idx_git_repository_sources_refresh
    ON context69.git_repository_sources (
        enabled,
        refresh_dirty,
        interval_seconds,
        last_refresh_attempt_at
    );

CREATE INDEX IF NOT EXISTS idx_git_repository_sources_deletion
    ON context69.git_repository_sources (deletion_requested_at)
    WHERE deletion_requested_at IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_git_repository_generations_cleanup
    ON context69.git_repository_generations (cleanup_pending, status, updated_at)
    WHERE cleanup_pending OR status IN ('superseded', 'failed');
