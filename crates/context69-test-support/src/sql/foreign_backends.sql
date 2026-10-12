-- Other backends attached to the target database. A reset while an
-- application connection is live would destroy in-flight work, so the runner
-- refuses rather than guessing that the session is harmless. The runner's own
-- pool is excluded by the application name it sets on connect.
SELECT application_name,
       state
FROM pg_stat_activity
WHERE datname = current_database()
  AND pid <> pg_backend_pid()
  AND backend_type = 'client backend'
  AND application_name <> 'context69-test-runner'
ORDER BY pid;