-- Locks the item row of a captured task grant and verifies that grant.
--
-- The row is locked BEFORE the grant is compared, so the decision is taken on
-- the row as it is after any concurrent transition waited for. A revoke, a
-- finish, a cancel or a reclaim that already moved the row therefore refuses
-- this write instead of racing it. The lease deadline is compared against the
-- clock of the database, never against the worker's clock.
--
-- `$1` is the item, `$2` the fencing token of the library attempt, `$3` the
-- library task of the segment and `$4` the execution generation.
SELECT item.id
FROM context69.task_items item
WHERE item.id = $1
  AND item.lease_token = $2
  AND item.status = 'running'
  AND item.lease_until > clock_timestamp()
  AND item.execution_task_id = $3
  AND item.execution_generation = $4
FOR UPDATE OF item
