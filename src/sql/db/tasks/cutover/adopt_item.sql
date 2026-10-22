-- Links a fresh execution segment to one unlinked queued or waiting item
-- (issue 1020 P4A).
--
-- The handover only ever links an item the library does not execute yet and
-- that the engine has not started: `queued` or `waiting`. A running item is
-- never adopted, and a terminal one is never resurrected. The domain status is
-- preserved exactly - a waiting item stays waiting, so a new library segment
-- alone never bypasses the domain admission of its parent, and the eligibility
-- rules still exclude a Docling wait until the existing reconciliation makes it
-- eligible. The retry deadline, attempt count, dependency key, resource, stage
-- and Docling reference all stay untouched.
--
-- `$1` is the item, `$2` the fresh library task and `$3` the generation.
UPDATE context69.task_items item
SET execution_task_id = $2,
    execution_generation = $3,
    updated_at = now()
WHERE item.id = $1
  AND item.execution_task_id IS NULL
  AND item.status IN ('queued', 'waiting')
RETURNING item.id
