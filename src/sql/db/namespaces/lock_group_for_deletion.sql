-- Locks the group row before its Git-owned state is inspected (issue #28
-- phase P13).
--
-- The lock is taken first and held for the same transaction that counts Git
-- ownership and deletes the group, so a concurrent group mutation cannot
-- interleave between the safety check and the delete.
--
-- $1 group_id.
SELECT id
FROM context69.groups
WHERE id = $1
FOR UPDATE
