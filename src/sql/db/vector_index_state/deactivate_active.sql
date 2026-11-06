-- Return the active pointer to no live collection.
--
-- An activation deactivates the previous active row in one statement and
-- activates the exact target in a second one, so the partial unique index never
-- observes two active rows.
UPDATE context69.vector_index_state
SET active = FALSE
WHERE active
