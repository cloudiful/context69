-- Whether the recorded owner of the rebuild barrier can still write.
--
-- The owner of a rebuild barrier is a fenced task item, and the recorded token
-- is the grant that owner held. The owner is live only while the item still
-- carries that exact token and still runs: a reclaimed item carries a fresh
-- token, and a terminal or deleted item cannot pass the grant check at all.
--
-- The lease deadline is deliberately not part of the predicate. An elapsed
-- deadline is not proof that the owner is gone: only a superseded token or a
-- terminal item is.
SELECT EXISTS (
    SELECT 1
    FROM context69.task_items
    WHERE id = $1
      AND status = 'running'
      AND lease_token = $2
) AS "live!"
