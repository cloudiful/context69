CREATE TABLE IF NOT EXISTS context69.library_folders (
    id UUID PRIMARY KEY,
    parent_id UUID REFERENCES context69.library_folders(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (btrim(name) <> '')
);

CREATE UNIQUE INDEX IF NOT EXISTS uq_library_folders_parent_name
    ON context69.library_folders ((COALESCE(parent_id::text, '__root__')), name);

CREATE TABLE IF NOT EXISTS context69.library_files (
    id UUID PRIMARY KEY,
    folder_id UUID REFERENCES context69.library_folders(id) ON DELETE CASCADE,
    filename TEXT NOT NULL,
    media_type TEXT NOT NULL,
    size_bytes BIGINT NOT NULL,
    sha256 TEXT NOT NULL,
    storage_rel_path TEXT NOT NULL,
    ingest_status TEXT NOT NULL,
    error_message TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    ingested_at TIMESTAMPTZ,
    CHECK (size_bytes >= 0),
    CHECK (btrim(filename) <> ''),
    CHECK (btrim(storage_rel_path) <> '')
);

CREATE UNIQUE INDEX IF NOT EXISTS uq_library_files_folder_filename
    ON context69.library_files ((COALESCE(folder_id::text, '__root__')), filename);

CREATE INDEX IF NOT EXISTS idx_library_files_folder_id
    ON context69.library_files (folder_id, filename);

CREATE TABLE IF NOT EXISTS context69.library_ingest_jobs (
    id UUID PRIMARY KEY,
    file_id UUID NOT NULL REFERENCES context69.library_files(id) ON DELETE CASCADE,
    status TEXT NOT NULL,
    docling_task_id TEXT,
    error_message TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_library_ingest_jobs_file_id
    ON context69.library_ingest_jobs (file_id, created_at DESC);

CREATE TABLE IF NOT EXISTS context69.library_file_documents (
    file_id UUID NOT NULL REFERENCES context69.library_files(id) ON DELETE CASCADE,
    document_id BIGINT NOT NULL REFERENCES context69.documents(id) ON DELETE CASCADE,
    section_key TEXT NOT NULL,
    section_label TEXT NOT NULL,
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (file_id, document_id),
    UNIQUE (file_id, section_key)
);

CREATE INDEX IF NOT EXISTS idx_library_file_documents_document_id
    ON context69.library_file_documents (document_id);
