-- The file of one running task item (issue 1020 P2).
--
-- The guard is the captured execution of the item: the fencing token of the
-- library attempt, the library task and the execution generation, compared
-- after the row lock was taken and against the clock of the database.
--
-- `$1` is the item, `$2` the fencing token, `$3` the file, `$4` the library
-- task and `$5` the execution generation.
UPDATE context69.task_items
SET file_id = $3,
    resource_id = $3::uuid::text,
    input_storage_object_id = NULL,
    updated_at = now()
WHERE id = $1
  AND lease_token = $2
  AND status = 'running'
  AND lease_until > clock_timestamp()
  AND execution_task_id = $4
  AND execution_generation = $5
