-- The vector identity an entry was admitted under.
--
-- An entry that admits new vector writes records the embedding fingerprint
-- and the collection it was admitted with. A metadata-only tombstone entry
-- creates no point and records neither, so both columns are null there.
SELECT embedding_fingerprint, collection_name
FROM context69.search_registry_entries
WHERE run_id = $1
