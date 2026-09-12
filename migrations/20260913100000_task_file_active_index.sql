-- Deduplicate file processing by file_id: support the active-file lookup that
-- rejects a second live processing item for one file. Partial index keeps the
-- write cost limited to the small active set. Idempotent and additive.
CREATE INDEX IF NOT EXISTS idx_task_items_active_file
    ON context69.task_items (file_id)
    WHERE file_id IS NOT NULL
      AND status IN ('queued', 'running', 'waiting');
