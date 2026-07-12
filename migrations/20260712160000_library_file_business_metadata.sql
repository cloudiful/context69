ALTER TABLE context69.library_files
    ADD COLUMN source_uri TEXT,
    ADD COLUMN published_at TIMESTAMPTZ,
    ADD COLUMN metadata_json JSONB NOT NULL DEFAULT '{}'::jsonb;

ALTER TABLE context69.library_file_documents
    ADD COLUMN section_external_id TEXT,
    ADD COLUMN section_source_uri TEXT,
    ADD COLUMN section_published_at TIMESTAMPTZ,
    ADD COLUMN section_metadata_json JSONB NOT NULL DEFAULT '{}'::jsonb;
