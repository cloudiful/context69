-- Tries the per-repository index lifetime lock without waiting.
--
-- Cleanup uses this so it defers instead of blocking behind an active index
-- run, then retries on a later maintenance pass. The key is distinct from the
-- short submission admission lock.
--
-- $1 lock key.
SELECT pg_try_advisory_xact_lock(hashtextextended($1, 0)) AS acquired
