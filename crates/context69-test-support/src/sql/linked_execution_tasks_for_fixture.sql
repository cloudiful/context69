-- Library tasks linked to one bounded fixture execution scope.
--
-- The link is the same precise one the reset cleanup uses: a library task in
-- the Context69 namespace whose payload item id equals an owned
-- `context69.task_items` row. This query restricts that link further to the
-- caller's fixture:
--
--   * exact fixture parent task ids ($1), or
--   * exact fixture groups ($3) fenced by their owner user id ($2), so a group
--     id alone can never select another owner's task.
--
-- Empty arrays and a NULL owner select nothing, so an empty scope can never
-- broaden to every task. The namespace is fixed, never caller supplied.
-- $4 is the last task id of the previous page (NULL starts at the beginning),
-- $5 the bounded page size. Library history is never deleted here.
SELECT task.namespace,
       task.queue,
       task.id AS task_id
FROM cloudiful_tasks.tasks task
JOIN context69.task_items item
  ON item.id::text = task.payload->>'item_id'
JOIN context69.tasks parent
  ON parent.id = item.task_id
WHERE task.namespace = 'context69'
  AND (
      parent.id = ANY($1::uuid[])
      OR (
          $2::bigint IS NOT NULL
          AND parent.user_id = $2
          AND parent.group_id = ANY($3::bigint[])
      )
  )
  AND ($4::uuid IS NULL OR task.id > $4::uuid)
ORDER BY task.id
LIMIT $5;
