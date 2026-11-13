-- Remove the global registry id capacity cap.
--
-- The per-file and per-run caps stay in effect, including their ordering, so a
-- rebuild still bounds one file and one ingestion run. The global cap could no
-- longer admit an installation whose retained registry entries already exceed
-- it, while the per-file and per-run reservations remained valid.
--
-- The constraints are replaced before the column is dropped because both
-- referenced it. No rows need backfilling: existing per-file and per-run values
-- were already valid under the retained ordering.
ALTER TABLE context69.runtime_file_library_settings
    DROP CONSTRAINT runtime_file_library_registry_caps_ordered,
    DROP CONSTRAINT runtime_file_library_registry_caps_positive,
    DROP COLUMN registry_max_ids_total;

ALTER TABLE context69.runtime_file_library_settings
    ADD CONSTRAINT runtime_file_library_registry_caps_positive
        CHECK (registry_max_ids_per_file > 0 AND registry_max_ids_per_run > 0),
    ADD CONSTRAINT runtime_file_library_registry_caps_ordered
        CHECK (registry_max_ids_per_run <= registry_max_ids_per_file);
