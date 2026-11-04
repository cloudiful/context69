-- Removes the test-only fault trigger, function and guard table
-- (issue 1172 P6A2). No business row is touched.
DROP TRIGGER IF EXISTS task_admission_fault_guard_trigger ON context69.task_items;
DROP FUNCTION IF EXISTS context69.task_admission_fault_guard_fn();
DROP TABLE IF EXISTS context69.task_admission_fault_guard;
