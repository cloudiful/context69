-- Remove automatic task-history retention (issue 391 Task 1).
-- Task history is never auto-deleted: completed/trash rows survive until the
-- user explicitly clears them via per-task trash/restore/delete or the future
-- user-scoped clear API. This migration only drops the unused retention
-- configuration and its scan index; it never deletes tasks, task_items, or
-- task_attempts rows.

DROP INDEX IF EXISTS context69.idx_tasks_terminal_cutoff;
DROP TABLE IF EXISTS context69.task_maintenance_settings;
