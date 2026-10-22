-- The library segment of one queued task item (issue 1020 P2).
--
-- The link names the library task that executes the item and the generation
-- the segment belongs to. Only a queued item takes a new link, so a running
-- item never points at a segment it does not hold.
UPDATE context69.task_items
SET execution_task_id = $2,
    execution_generation = $3
WHERE id = $1
  AND status = 'queued'
RETURNING id
