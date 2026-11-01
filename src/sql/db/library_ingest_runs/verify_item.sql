SELECT item.id
FROM context69.task_items item
WHERE item.id = $1
  AND item.task_id = $2
  AND item.file_id = $3
  AND item.status = 'running'
  AND item.lease_token = $4
  AND item.lease_until > clock_timestamp()
  AND item.execution_task_id = $5
  AND item.execution_generation = $6
FOR UPDATE OF item
