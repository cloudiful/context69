-- The database name the server actually reports (issue 1020 P4A).
--
-- The handover tool compares this with the name configured in the connection
-- string and with --expect-database before it reads or writes anything, so a
-- tool pointed at the wrong database is refused instead of acting on it.
SELECT current_database() AS "name!"
