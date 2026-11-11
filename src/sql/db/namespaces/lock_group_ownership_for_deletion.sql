-- Exclusive ownership lock for one group delete (issue #28 phase P13).
--
-- Source registration takes shared locks on the target group key and every
-- ancestor key in the same transaction as its insert. Taking the exclusive lock
-- on this group before its row lock means a concurrent registration either
-- commits first and is counted, or waits and then fails once the group is gone.
-- The advisory lock is deliberately acquired before the row lock so a
-- registration that already holds the shared lock can finish its insert instead
-- of deadlocking against the delete's row lock.
--
-- $1 group_id.
SELECT pg_advisory_xact_lock(
    hashtextextended('context69.git_group_ownership:' || $1::bigint::text, 0)
)
