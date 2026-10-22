-- Linkage between a domain task item and the durable library execution segment
-- that runs it (issue #1020 P1).
--
-- Additive only: no existing column is reinterpreted, no row is rewritten, and
-- no domain record is replaced. Both columns stay nullable, so every row that
-- predates the library adoption keeps its history exactly as it was, and the
-- generation counter starts at zero, so an item that has never been given an
-- execution segment stays distinguishable from one that has.
--
-- task_items.execution_task_id is the library task that owns the current
-- execution segment of the item, and task_items.execution_generation counts
-- the segments the item has been given. task_attempts keeps the two immutable
-- identities of the library attempt behind a domain attempt, so the domain
-- history and the library history can be correlated without a lookup of the
-- latest owner.

ALTER TABLE context69.task_items
    ADD COLUMN IF NOT EXISTS execution_task_id UUID,
    ADD COLUMN IF NOT EXISTS execution_generation BIGINT NOT NULL DEFAULT 0;

ALTER TABLE context69.task_attempts
    ADD COLUMN IF NOT EXISTS execution_task_id UUID,
    ADD COLUMN IF NOT EXISTS execution_attempt_id UUID;

CREATE INDEX IF NOT EXISTS idx_task_items_execution_task_id
    ON context69.task_items (execution_task_id)
    WHERE execution_task_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_task_attempts_execution_task_id
    ON context69.task_attempts (execution_task_id)
    WHERE execution_task_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_task_attempts_execution_attempt_id
    ON context69.task_attempts (execution_attempt_id)
    WHERE execution_attempt_id IS NOT NULL;
