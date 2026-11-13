-- The registry capacity policy of the file library.
--
-- The admission reads this row inside its own transaction, under the registry
-- epoch lock, so the caps it applies are the ones the deployment persisted at
-- that moment. A deployment without a runtime file-library row has no
-- persisted policy yet; the admission then uses the documented defaults, which
-- are the values the configuration seeds and the upgrade migration installs.
SELECT
    registry_max_ids_per_file,
    registry_max_ids_per_run
FROM context69.runtime_file_library_settings
WHERE singleton = TRUE
