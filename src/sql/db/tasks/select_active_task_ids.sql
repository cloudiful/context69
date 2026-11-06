-- The candidate task ids of one bulk cancellation (issue 1212 R2).
--
-- The set is selected once, without locking, inside the cancellation
-- transaction. The caller then locks/cancels the items of exactly these tasks
-- before taking the registry epoch and cancelling exactly these task rows.
-- That keeps the bulk path on the canonical item -> epoch -> task order that
-- activation and paused resume already use, and a task that completes between
-- the selection and the update is simply not eligible when its row is updated.
--
-- Ordered ids make any two bulk cancellations acquire the same task rows in
-- one deterministic order.
SELECT id
FROM context69.tasks
WHERE status IN ('queued', 'running', 'waiting')
ORDER BY id
