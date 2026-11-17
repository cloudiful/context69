ALTER TABLE context69.task_items
    ADD COLUMN released_attempts INTEGER NOT NULL DEFAULT 0;

ALTER TABLE context69.task_items
    ADD CONSTRAINT task_items_released_attempts_check
    CHECK (released_attempts >= 0 AND paused_attempts + released_attempts <= attempt_count);
