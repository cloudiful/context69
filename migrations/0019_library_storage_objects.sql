CREATE TABLE IF NOT EXISTS context69.library_storage_objects (
    id UUID PRIMARY KEY,
    group_id BIGINT NOT NULL REFERENCES context69.groups(id) ON DELETE CASCADE,
    sha256 TEXT NOT NULL,
    size_bytes BIGINT NOT NULL,
    storage_backend TEXT NOT NULL,
    object_key TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (length(sha256) = 64),
    CHECK (size_bytes >= 0),
    CHECK (storage_backend IN ('local', 's3')),
    CHECK (btrim(object_key) <> ''),
    UNIQUE (group_id, sha256)
);

ALTER TABLE context69.library_files
    ADD COLUMN IF NOT EXISTS storage_object_id UUID
        REFERENCES context69.library_storage_objects(id) ON DELETE RESTRICT;

CREATE INDEX IF NOT EXISTS idx_library_files_storage_object_id
    ON context69.library_files (storage_object_id)
    WHERE storage_object_id IS NOT NULL;

ALTER TABLE context69.runtime_file_library_settings
    ADD COLUMN IF NOT EXISTS s3_endpoint TEXT,
    ADD COLUMN IF NOT EXISTS s3_region TEXT,
    ADD COLUMN IF NOT EXISTS s3_bucket TEXT,
    ADD COLUMN IF NOT EXISTS s3_prefix TEXT,
    ADD COLUMN IF NOT EXISTS s3_path_style BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN IF NOT EXISTS s3_access_key TEXT,
    ADD COLUMN IF NOT EXISTS s3_secret_key TEXT;
