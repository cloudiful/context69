-- Takes the per-repository index lifetime lock for the caller's transaction.
--
-- One pool connection holds this transaction-scoped advisory lock for the whole
-- index lifetime, so generation and source cleanup can observe that a writer is
-- still in use instead of trusting an age threshold. The key is deliberately
-- distinct from the short submission admission lock.
--
-- $1 lock key.
SELECT pg_advisory_xact_lock(hashtextextended($1, 0))
