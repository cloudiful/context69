-- Soft-delete (trash) for task history. `deleted_at` is the only trash marker:
-- a non-null value means the row sits in the user's recycle bin and is excluded
-- from active task lists. Trashing never cancels work, deletes file records,
-- processed text, or vectors; those are owned elsewhere and are untouched here.

ALTER TABLE context69.tasks
    ADD COLUMN IF NOT EXISTS deleted_at TIMESTAMPTZ;

-- Trash listing and retention cleanup only ever scan trashed rows.
CREATE INDEX IF NOT EXISTS idx_tasks_trashed_deleted_at
    ON context69.tasks (deleted_at)
    WHERE deleted_at IS NOT NULL;
