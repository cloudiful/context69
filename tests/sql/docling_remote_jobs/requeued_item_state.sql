-- Static readback for the legacy-adoption immediacy check
-- (reconcile_adoption_preserves_payload_and_marks_due).
-- requeue_waiting_item.sql sets next_attempt_at and updated_at to the same
-- now() in one statement, so the test proves immediate scheduling by exact
-- equality on the database clock. $1 is the task item id.
SELECT status, waiting_reason, payload, next_attempt_at, updated_at
FROM context69.task_items
WHERE id = $1
