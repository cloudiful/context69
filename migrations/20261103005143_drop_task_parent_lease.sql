-- Remove the parent-task admission lease (issue 1172 P2).
--
-- Execution capacity is the durable library attempt admission: the queue policy
-- stored by `cloudiful-tasks` is the only owner of how many attempts run at
-- once, and every replica shares it. The domain parent no longer reserves a
-- slot, so its own lease columns and the index over them carry no authority and
-- must not survive as a second capacity counter.
--
-- Only the parent lease is removed. `context69.task_items.lease_token` and
-- `context69.task_items.lease_until` are the item lease that fences a worker,
-- together with `idx_task_items_status_lease`, and they stay. No CASCADE and no
-- IF EXISTS: an unexpected shape must fail the migration instead of being
-- hidden.
DROP INDEX context69.idx_tasks_status_lease;

ALTER TABLE context69.tasks
    DROP COLUMN lease_until,
    DROP COLUMN lease_token;
