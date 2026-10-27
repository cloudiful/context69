-- The bounded selection of the unlinked running items whose lease is gone
-- (issue 1100 P1).
--
-- The filter is applied in SQL before the page bound, so any number of older
-- linked, terminal or still-leased rows can never starve an eligible item out
-- of the batch. The caller runs this inside the handover transaction under the
-- admission lock, then locks the selected ids in its deterministic order and
-- rechecks every row before it closes it: a running item the library does not
-- execute, whose domain mirror lease is already gone.
--
-- A live lease is never selected here, and the batch refuses every write while
-- either authority still holds one.
--
-- `$1` is the batch bound (at most MAX_HANDOVER_ITEMS).
SELECT item.id
FROM context69.task_items item
WHERE item.execution_task_id IS NULL
  AND item.status = 'running'
  AND (item.lease_until IS NULL OR item.lease_until <= clock_timestamp())
ORDER BY item.created_at, item.id
LIMIT $1
