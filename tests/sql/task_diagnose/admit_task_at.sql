-- Admit one parent at a caller-supplied instant so the admission-age
-- regression never depends on the database clock agreeing with the host clock.
-- $1 is the task id, $2 the explicit admission instant. The lease deadline
-- stays on the database clock, so the fixture still proves that a future lease
-- value is not mistaken for the admission time.
UPDATE context69.tasks
SET status = 'running',
    started_at = $2,
    lease_token = gen_random_uuid(),
    lease_until = now() + interval '8 minutes'
WHERE id = $1
