-- Take the session advisory lock, waiting until the other runner finishes its
-- reset/migrate/test/cleanup sequence.
SELECT pg_advisory_lock($1, $2);