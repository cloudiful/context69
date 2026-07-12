ALTER TABLE context69.library_url_import_jobs
    ADD COLUMN translation_provided BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN translation_source_locale TEXT,
    ADD COLUMN translation_target_locales TEXT[] NOT NULL DEFAULT ARRAY[]::TEXT[];
