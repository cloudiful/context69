-- Make one recorded physical collection the active vector index.
--
-- Only a row that already records a verified identity and its actual dimension
-- can be activated, and the partial unique index refuses a second active row.
UPDATE context69.vector_index_state
SET active = TRUE
WHERE collection_name = $1
