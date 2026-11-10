-- Bounded fair retry of pending Git source deletions (issue #28 phase P9).
--
-- A failed source deletion records how often it was attempted and when it may
-- be selected again. The batch order is due-time first, so backed-off permanent
-- failures cannot occupy every bounded pass and starve a later recoverable
-- deletion. The backoff itself is exponential and capped in the marking
-- statement.

ALTER TABLE context69.git_repository_sources
    ADD COLUMN deletion_attempts INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN deletion_attempted_at TIMESTAMPTZ,
    ADD COLUMN deletion_next_retry_at TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS idx_git_repository_sources_deletion_retry
    ON context69.git_repository_sources (deletion_next_retry_at, repository_key)
    WHERE deletion_requested_at IS NOT NULL;
