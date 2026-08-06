-- Singleton runtime settings for task history maintenance. Values take effect
-- immediately; the background cleanup reads them at the start of every cycle.
CREATE TABLE IF NOT EXISTS context69.task_maintenance_settings (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE,
    cleanup_enabled BOOLEAN NOT NULL DEFAULT TRUE,
    retention_days BIGINT NOT NULL DEFAULT 30,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (singleton),
    CHECK (retention_days BETWEEN 1 AND 3650)
);

INSERT INTO context69.task_maintenance_settings (singleton)
VALUES (TRUE)
ON CONFLICT (singleton) DO NOTHING;

-- Terminal-task history cleanup scans by COALESCE(finished_at, updated_at);
-- the partial index keeps the scan and the SKIP LOCKED batch selection on
-- terminal rows instead of the whole task table.
CREATE INDEX IF NOT EXISTS idx_tasks_terminal_cutoff
    ON context69.tasks (COALESCE(finished_at, updated_at))
    WHERE status IN ('succeeded', 'failed', 'cancelled');
