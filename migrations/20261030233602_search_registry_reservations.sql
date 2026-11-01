ALTER TABLE context69.search_registry_entries
    ADD COLUMN reserved_ids BIGINT NOT NULL DEFAULT 0 CHECK (reserved_ids >= 0);

UPDATE context69.search_registry_entries
SET reserved_ids = cardinality(staged_ids)::bigint + cardinality(retired_ids)::bigint;
