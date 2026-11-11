-- Fair, bounded retry of pending Git generation vector cleanup (issue #28
-- phase P7).
--
-- A failed cleanup records how often it was attempted and when it may be
-- selected again. The next maintenance pass orders the due sources by that
-- timestamp, so a bounded batch can never keep selecting the same permanently
-- failing sources and starve a later recoverable one. The backoff itself is
-- exponential and capped in the marking statement.

ALTER TABLE context69.git_repository_generations
    ADD COLUMN cleanup_attempts INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN cleanup_next_retry_at TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS idx_git_repository_generations_cleanup_retry
    ON context69.git_repository_generations (cleanup_next_retry_at, repository_key)
    WHERE cleanup_pending;
