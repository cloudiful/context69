-- Take the session advisory lock without blocking, so a second runner refuses
-- immediately instead of waiting on an active reset.
SELECT pg_try_advisory_lock($1, $2);