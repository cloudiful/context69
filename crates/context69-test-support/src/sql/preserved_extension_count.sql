-- Extensions installed in the target database. They are reported as a count
-- and preserved: a reset never drops or recreates one.
SELECT count(*)::bigint
FROM pg_extension;