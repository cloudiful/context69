-- A claimed item: the lease of the library attempt is live, so the fixture
-- carries the token and the deadline the claim would have mirrored.
INSERT INTO context69.task_items
    (id, task_id, ordinal, payload, status, lease_token, lease_until, attempt_count)
VALUES ($1, $2, 0, $3, 'running', $4, now() + interval '5 minutes', 1)
