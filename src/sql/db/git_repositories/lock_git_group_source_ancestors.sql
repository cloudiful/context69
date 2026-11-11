-- Full ancestor row-lock chain for one source registration (issue #28 phase
-- P13).
--
-- The shared advisory locks serialize participants that use the ownership key
-- protocol. These `FOR KEY SHARE` row locks are the independent control: a
-- group delete takes `FOR UPDATE` on its own group row, which conflicts with
-- this lock on every ancestor row. A registration into a descendant therefore
-- cannot commit while an ancestor is being deleted, and an ancestor delete
-- cannot pass its row lock until an in-flight registration commits.
--
-- Locks are taken on the target group and every ancestor in the same
-- registration transaction, after the shared advisory locks.
--
-- $1 group_id.
WITH RECURSIVE ancestors AS (
    SELECT g.id, g.parent_group_id
    FROM context69.groups g
    WHERE g.id = $1::bigint

    UNION ALL

    SELECT parent.id, parent.parent_group_id
    FROM context69.groups parent
    JOIN ancestors child ON child.parent_group_id = parent.id
)
SELECT g.id
FROM context69.groups g
JOIN ancestors ancestor ON ancestor.id = g.id
ORDER BY g.id
FOR KEY SHARE OF g
