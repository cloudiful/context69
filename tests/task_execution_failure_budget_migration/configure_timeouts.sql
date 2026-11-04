-- Bound the rolled-back upgrade fixture so its ACCESS EXCLUSIVE lock cannot
-- wait indefinitely behind, or block the rest of the suite for, another session.
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '30s';
