-- Library tasks precisely linked to the Context69 items a reset is about to
-- delete.
--
-- The link is the segment payload's item id against the owned domain item id,
-- restricted to the Context69 namespace. A task of another namespace, or one
-- whose payload is not an owned item, is never selected. The queue name is
-- returned per task so the caller opens the queue each fact actually belongs
-- to instead of assuming the current queue layout.
--
-- $1 is the last task id of the previous page (NULL starts at the beginning),
-- $2 the bounded page size. Ordering by id makes the pages deterministic while
-- the caller only reads; library history is never deleted here.
SELECT task.namespace,
       task.queue,
       task.id AS task_id
FROM cloudiful_tasks.tasks task
JOIN context69.task_items item
  ON item.id::text = task.payload->>'item_id'
WHERE task.namespace = 'context69'
  AND ($1::uuid IS NULL OR task.id > $1::uuid)
ORDER BY task.id
LIMIT $2;
