-- Arms the guard for exactly one test-owned item and mode (issue 1172 P6A2).
INSERT INTO context69.task_admission_fault_guard (item_id, mode)
VALUES ($1, $2)
ON CONFLICT (item_id) DO UPDATE SET mode = EXCLUDED.mode
RETURNING item_id
