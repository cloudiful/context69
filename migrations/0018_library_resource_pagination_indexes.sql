CREATE INDEX IF NOT EXISTS idx_library_files_group_folder_updated_at
    ON context69.library_files (group_id, folder_id, updated_at DESC, id);

CREATE INDEX IF NOT EXISTS idx_library_files_group_folder_size
    ON context69.library_files (group_id, folder_id, size_bytes DESC, id);

CREATE INDEX IF NOT EXISTS idx_library_files_group_folder_status
    ON context69.library_files (group_id, folder_id, ingest_status, id);

CREATE INDEX IF NOT EXISTS idx_library_folders_group_parent_updated_at
    ON context69.library_folders (group_id, parent_id, updated_at DESC, id);
