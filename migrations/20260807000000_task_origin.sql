ALTER TABLE context69.tasks
    ADD COLUMN origin TEXT NOT NULL DEFAULT 'manual';

CREATE INDEX IF NOT EXISTS idx_tasks_origin_created_at
    ON context69.tasks (origin, created_at DESC);
