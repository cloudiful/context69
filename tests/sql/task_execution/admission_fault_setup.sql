-- Test-only fault guard (issue 1172 P6A2). It is created only by an ignored
-- test on the issue-owned database and dropped by admission_fault_cleanup.sql.
CREATE TABLE IF NOT EXISTS context69.task_admission_fault_guard (
    item_id uuid PRIMARY KEY,
    mode text NOT NULL CHECK (mode IN ('mirror', 'relink'))
);

CREATE OR REPLACE FUNCTION context69.task_admission_fault_guard_fn()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.status = 'running'
       AND OLD.status IS DISTINCT FROM 'running'
       AND EXISTS (
           SELECT 1 FROM context69.task_admission_fault_guard
           WHERE item_id = NEW.id AND mode = 'mirror'
       )
    THEN
        RETURN NULL;
    END IF;
    IF NEW.execution_task_id IS DISTINCT FROM OLD.execution_task_id
       AND EXISTS (
           SELECT 1 FROM context69.task_admission_fault_guard
           WHERE item_id = NEW.id AND mode = 'relink'
       )
    THEN
        RETURN NULL;
    END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS task_admission_fault_guard_trigger ON context69.task_items;
CREATE TRIGGER task_admission_fault_guard_trigger
BEFORE UPDATE ON context69.task_items
FOR EACH ROW
EXECUTE FUNCTION context69.task_admission_fault_guard_fn();
