ALTER TABLE context69.library_files
    ADD COLUMN IF NOT EXISTS external_id TEXT;

CREATE UNIQUE INDEX IF NOT EXISTS uq_library_files_project_external_id
    ON context69.library_files (project_id, external_id)
    WHERE external_id IS NOT NULL;
