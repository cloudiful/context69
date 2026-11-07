-- Cooperative per-task pause state.
--
-- The pause state is independent of the task status: a queued task stays
-- queued while paused, and a running task first enters `pausing` until its
-- current attempt parks at a safe checkpoint. `paused_at` records when the
-- pause last became effective; it is cleared on resume.
ALTER TABLE context69.tasks
    ADD COLUMN pause_state TEXT NOT NULL DEFAULT 'unpaused',
    ADD COLUMN paused_at TIMESTAMPTZ;

ALTER TABLE context69.tasks
    ADD CONSTRAINT tasks_pause_state_check
        CHECK (pause_state IN ('unpaused', 'pausing', 'paused'));

-- Attempts that parked at a cooperative pause checkpoint do not spend the
-- failure budget. The effective usage is `attempt_count - paused_attempts`.
ALTER TABLE context69.task_items
    ADD COLUMN paused_attempts INTEGER NOT NULL DEFAULT 0;

ALTER TABLE context69.task_items
    ADD CONSTRAINT task_items_paused_attempts_check
        CHECK (paused_attempts >= 0 AND paused_attempts <= attempt_count);
