-- Durable global task-queue admission control (issue 1212 P2).
--
-- The queue needs one deployment-wide pause that survives a service restart and
-- is visible to every replica. It is a singleton row, so a pause and a claim
-- serialize on the same durable state, and the row always exists: an
-- installation that predates this migration starts unpaused.
CREATE TABLE context69.task_queue_control (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    paused BOOLEAN NOT NULL DEFAULT FALSE,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

INSERT INTO context69.task_queue_control (id, paused)
VALUES (1, FALSE);
