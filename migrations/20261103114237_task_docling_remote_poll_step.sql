ALTER TABLE context69.task_docling_remote_jobs
    ADD COLUMN poll_step INTEGER NOT NULL DEFAULT 0
    CHECK (poll_step >= 0);
