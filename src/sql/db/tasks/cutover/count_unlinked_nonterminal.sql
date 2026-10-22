-- Unlinked non-terminal task items (issue 1020 P4A).
--
-- An item that the library does not execute yet, and that is neither terminal
-- nor linked to an execution segment, is work the pre-cutover engine may still
-- own. The startup latch counts them: while any exists, the task engine must
-- not start, because starting would run a second execution authority beside the
-- abandoned rows.
SELECT count(*) AS "count!"
FROM context69.task_items item
WHERE item.execution_task_id IS NULL
  AND item.status IN ('queued', 'waiting', 'running')
