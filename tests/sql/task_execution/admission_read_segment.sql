-- Reads the admission metadata of one owned library segment (issue 1172 P6A2).
SELECT id, status, class, resources
FROM cloudiful_tasks.tasks
WHERE id = $1
  AND namespace = 'context69'
  AND queue = 'items'
