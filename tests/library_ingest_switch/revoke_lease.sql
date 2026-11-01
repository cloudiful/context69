-- Move the lease of a task item to another token, as a reclaiming segment does.
UPDATE context69.task_items
SET lease_token = $1
WHERE id = $2
