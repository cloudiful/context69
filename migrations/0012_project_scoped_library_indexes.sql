DROP INDEX IF EXISTS context69.uq_library_folders_parent_name;
DROP INDEX IF EXISTS context69.uq_library_files_folder_filename;

CREATE UNIQUE INDEX IF NOT EXISTS uq_library_folders_project_parent_name
    ON context69.library_folders (project_id, (COALESCE(parent_id::text, '__root__')), name);

CREATE UNIQUE INDEX IF NOT EXISTS uq_library_files_project_folder_filename
    ON context69.library_files (project_id, (COALESCE(folder_id::text, '__root__')), filename);
