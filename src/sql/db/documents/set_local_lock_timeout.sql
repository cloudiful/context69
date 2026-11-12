-- Bound how long the one-time derived-index reset waits for a table lock.
--
-- The setting is transaction-local, so it disappears with the reset
-- transaction and never narrows the timeout of an unrelated statement.
SELECT set_config('lock_timeout', $1, true)
