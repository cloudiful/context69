ALTER TABLE context69.docling_settings
    ADD COLUMN IF NOT EXISTS task_timeout_secs BIGINT NOT NULL DEFAULT 600
    CHECK (task_timeout_secs > 0);
