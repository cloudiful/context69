-- Open task-attempt write-path indexes (issue #129 phase 3).
--
-- Finish/wait/release/progress/cancel/maintenance statements close open
-- attempts by `item_id` or `task_id` with a `finished_at IS NULL` guard,
-- but `task_attempts` has no index on either column, so every such update
-- scans the full attempt history. These two partial indexes cover only the
-- small open set and leave historical (finished) rows unindexed:
--   * `item_id` lookups for per-item finish/wait/release/progress paths;
--   * `task_id` lookups for task-wide fail/cancel paths.
-- Both indexes are purely additive and idempotent (`IF NOT EXISTS`); no
-- rows are read or written, no task-state behavior changes, and no
-- historical attempt-ordering index is added.

CREATE INDEX IF NOT EXISTS idx_task_attempts_item_open
    ON context69.task_attempts (item_id)
    WHERE finished_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_task_attempts_task_open
    ON context69.task_attempts (task_id)
    WHERE finished_at IS NULL;
