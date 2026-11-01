-- The registry capacity policy of the file library.
--
-- The three caps bound how many vector point ids the search registry admits
-- for the whole deployment, for one file, and for one ingestion run. They are
-- cumulative: the usage counts every retained entry, including the tombstones
-- of permanently deleted entities, so the caps bound the retained work rather
-- than the live documents.
--
-- The defaults match the deployment configuration, so an upgraded installation
-- keeps the same policy a fresh boot would seed.
ALTER TABLE context69.runtime_file_library_settings
    ADD COLUMN registry_max_ids_total BIGINT NOT NULL DEFAULT 32768,
    ADD COLUMN registry_max_ids_per_file BIGINT NOT NULL DEFAULT 16384,
    ADD COLUMN registry_max_ids_per_run BIGINT NOT NULL DEFAULT 8192;

ALTER TABLE context69.runtime_file_library_settings
    ADD CONSTRAINT runtime_file_library_registry_caps_positive
        CHECK (registry_max_ids_total > 0 AND registry_max_ids_per_file > 0
            AND registry_max_ids_per_run > 0),
    ADD CONSTRAINT runtime_file_library_registry_caps_ordered
        CHECK (registry_max_ids_per_run <= registry_max_ids_per_file
            AND registry_max_ids_per_file <= registry_max_ids_total);
