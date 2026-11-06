-- Clear the recorded vector indices of a test deployment (issue 1212 P4a).
--
-- A fixture owns the shared schema, so it removes the previous identities
-- before it seeds its own. No collection of the disposable test database is
-- live afterwards.
DELETE FROM context69.vector_index_state
