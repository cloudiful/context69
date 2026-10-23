-- The database clock.
--
-- The guard under test compares against `statement_timestamp()`, so a case
-- places its rows relative to the database's own clock rather than to the
-- clock of the process that runs the test.
SELECT statement_timestamp() AS now
