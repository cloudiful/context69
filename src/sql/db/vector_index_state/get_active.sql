-- The one physical collection ordinary point writers may use.
--
-- The active row is the authority of the live vector space: its fingerprint is
-- the desired logical identity the deployment was configured with, and its
-- collection name is the physical collection that identity was built into.
SELECT collection_name, fingerprint, dimensions
FROM context69.vector_index_state
WHERE active
