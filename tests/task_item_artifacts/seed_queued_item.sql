-- A queued item, claimed through the library by the test fixture.
INSERT INTO context69.task_items
    (id, task_id, ordinal, payload, status, attempt_count)
VALUES ($1, $2, 0, $3, 'queued', 0)
