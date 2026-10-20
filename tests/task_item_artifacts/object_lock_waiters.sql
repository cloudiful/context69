-- Bounded lock-visibility probe (issue #975 P10): a backend that is actively
-- waiting because the transaction on $1 holds a conflicting row lock while
-- working on a storage object. The cleanup must be seen here before its
-- reference snapshot counts.
SELECT count(*)::bigint AS "count!"
FROM pg_stat_activity AS activity
WHERE activity.datname = current_database()
  AND activity.pid <> pg_backend_pid()
  AND activity.wait_event_type = 'Lock'
  AND $1 = ANY(pg_blocking_pids(activity.pid))
  AND activity.query LIKE '%library_storage_objects%'
