-- The staged input object of one running task item (issue 1020 P2).
--
-- The guard is the captured execution of the item: the fencing token of the
-- library attempt, the library task and the execution generation, compared
-- after the row lock was taken and against the clock of the database.
--
-- `$1` is the item, `$2` the fencing token, `$3` the object, `$4` the payload,
-- `$5` the library task and `$6` the execution generation.
UPDATE context69.task_items
SET input_storage_object_id = $3,
    payload = $4,
    updated_at = now()
WHERE id = $1
  AND lease_token = $2
  AND status = 'running'
  AND lease_until > clock_timestamp()
  AND execution_task_id = $5
  AND execution_generation = $6
