-- The file of one running task item without clearing its staged input.
--
-- A replacement keeps the item's staged source object attached until the
-- atomic switch commits. The guard is the captured execution of the item: the
-- fencing token of the library attempt, the library task and the execution
-- generation.
--
-- `$1` is the item, `$2` the fencing token, `$3` the file, `$4` the library
-- task and `$5` the execution generation.
UPDATE context69.task_items
SET file_id = $3,
    resource_id = $3::uuid::text,
    updated_at = now()
WHERE id = $1
  AND lease_token = $2
  AND status = 'running'
  AND lease_until > clock_timestamp()
  AND execution_task_id = $4
  AND execution_generation = $5
RETURNING id
