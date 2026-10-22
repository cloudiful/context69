-- Read-only inventory of one page of unlinked non-terminal task items
-- (issue 1020 P4A).
--
-- The page is bounded by the caller. Only execution metadata is selected: the
-- status, the parent kind, the ordinal, the execution generation and link, the
-- retry deadline and whether the item waits on the Docling dependency that
-- admission still excludes. The item payload, the user identity and every
-- resource name stay out of the tool.
SELECT item.id AS item_id,
       item.task_id,
       item.status,
       task.kind,
       item.ordinal,
       (task.deleted_at IS NOT NULL) AS "parent_retired!",
       item.execution_generation,
       item.execution_task_id,
       item.next_attempt_at,
       (item.status = 'waiting'
        AND COALESCE(item.waiting_reason, '') = 'docling') AS "docling_waiting!"
FROM context69.task_items item
JOIN context69.tasks task ON task.id = item.task_id
WHERE item.execution_task_id IS NULL
  AND item.status IN ('queued', 'waiting', 'running')
ORDER BY item.created_at, item.id
LIMIT $1 OFFSET $2
