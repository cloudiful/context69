-- Release the session advisory lock explicitly. Closing the connection also
-- releases it, so this is a courtesy rather than the safety mechanism.
SELECT pg_advisory_unlock($1, $2);