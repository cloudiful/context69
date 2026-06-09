CREATE TABLE IF NOT EXISTS context69.users (
    id BIGSERIAL PRIMARY KEY,
    login_name TEXT NOT NULL UNIQUE,
    display_name TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    is_admin BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (btrim(login_name) <> ''),
    CHECK (btrim(display_name) <> ''),
    CHECK (btrim(password_hash) <> '')
);

CREATE TABLE IF NOT EXISTS context69.groups (
    id BIGSERIAL PRIMARY KEY,
    parent_group_id BIGINT REFERENCES context69.groups(id) ON DELETE CASCADE,
    group_key TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    visibility TEXT NOT NULL,
    kind TEXT NOT NULL,
    owner_user_id BIGINT REFERENCES context69.users(id) ON DELETE SET NULL,
    created_by_user_id BIGINT REFERENCES context69.users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (btrim(group_key) <> ''),
    CHECK (btrim(name) <> ''),
    CHECK (visibility IN ('public', 'private')),
    CHECK (kind IN ('personal', 'shared'))
);

CREATE INDEX IF NOT EXISTS idx_groups_parent_group_id
    ON context69.groups (parent_group_id);

CREATE TABLE IF NOT EXISTS context69.group_memberships (
    group_id BIGINT NOT NULL REFERENCES context69.groups(id) ON DELETE CASCADE,
    user_id BIGINT NOT NULL REFERENCES context69.users(id) ON DELETE CASCADE,
    role TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (group_id, user_id),
    CHECK (role IN ('owner', 'maintainer', 'viewer'))
);

CREATE INDEX IF NOT EXISTS idx_group_memberships_user_id
    ON context69.group_memberships (user_id, group_id);

CREATE TABLE IF NOT EXISTS context69.projects (
    id BIGSERIAL PRIMARY KEY,
    group_id BIGINT NOT NULL REFERENCES context69.groups(id) ON DELETE CASCADE,
    project_key TEXT NOT NULL,
    name TEXT NOT NULL,
    visibility TEXT NOT NULL,
    created_by_user_id BIGINT REFERENCES context69.users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (group_id, project_key),
    CHECK (btrim(project_key) <> ''),
    CHECK (btrim(name) <> ''),
    CHECK (visibility IN ('public', 'private'))
);

CREATE INDEX IF NOT EXISTS idx_projects_group_id
    ON context69.projects (group_id, project_key);

CREATE TABLE IF NOT EXISTS context69.project_memberships (
    project_id BIGINT NOT NULL REFERENCES context69.projects(id) ON DELETE CASCADE,
    user_id BIGINT NOT NULL REFERENCES context69.users(id) ON DELETE CASCADE,
    role TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (project_id, user_id),
    CHECK (role IN ('owner', 'maintainer', 'viewer'))
);

CREATE INDEX IF NOT EXISTS idx_project_memberships_user_id
    ON context69.project_memberships (user_id, project_id);

CREATE TABLE IF NOT EXISTS context69.refresh_tokens (
    id UUID PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES context69.users(id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL UNIQUE,
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ,
    replaced_by_token_id UUID REFERENCES context69.refresh_tokens(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at TIMESTAMPTZ,
    CHECK (btrim(token_hash) <> '')
);

CREATE INDEX IF NOT EXISTS idx_refresh_tokens_user_id
    ON context69.refresh_tokens (user_id, expires_at DESC);

INSERT INTO context69.groups (
    group_key,
    name,
    visibility,
    kind
)
VALUES (
    'public',
    'Public',
    'public',
    'shared'
)
ON CONFLICT (group_key) DO NOTHING;

INSERT INTO context69.projects (
    group_id,
    project_key,
    name,
    visibility
)
SELECT
    g.id,
    'default-public',
    'Default Public Project',
    'public'
FROM context69.groups g
WHERE g.group_key = 'public'
ON CONFLICT (group_id, project_key) DO NOTHING;

ALTER TABLE context69.source_configs
    ADD COLUMN IF NOT EXISTS group_id BIGINT REFERENCES context69.groups(id) ON DELETE RESTRICT,
    ADD COLUMN IF NOT EXISTS project_id BIGINT REFERENCES context69.projects(id) ON DELETE RESTRICT,
    ADD COLUMN IF NOT EXISTS visibility TEXT,
    ADD CONSTRAINT chk_source_configs_visibility
        CHECK (visibility IS NULL OR visibility IN ('public', 'private'));

ALTER TABLE context69.source_checkpoints
    ADD COLUMN IF NOT EXISTS group_id BIGINT REFERENCES context69.groups(id) ON DELETE RESTRICT,
    ADD COLUMN IF NOT EXISTS project_id BIGINT REFERENCES context69.projects(id) ON DELETE RESTRICT,
    ADD COLUMN IF NOT EXISTS visibility TEXT,
    ADD CONSTRAINT chk_source_checkpoints_visibility
        CHECK (visibility IS NULL OR visibility IN ('public', 'private'));

ALTER TABLE context69.sync_runs
    ADD COLUMN IF NOT EXISTS group_id BIGINT REFERENCES context69.groups(id) ON DELETE RESTRICT,
    ADD COLUMN IF NOT EXISTS project_id BIGINT REFERENCES context69.projects(id) ON DELETE RESTRICT,
    ADD COLUMN IF NOT EXISTS visibility TEXT,
    ADD CONSTRAINT chk_sync_runs_visibility
        CHECK (visibility IS NULL OR visibility IN ('public', 'private'));

ALTER TABLE context69.documents
    ADD COLUMN IF NOT EXISTS group_id BIGINT REFERENCES context69.groups(id) ON DELETE RESTRICT,
    ADD COLUMN IF NOT EXISTS project_id BIGINT REFERENCES context69.projects(id) ON DELETE RESTRICT,
    ADD COLUMN IF NOT EXISTS visibility TEXT,
    ADD CONSTRAINT chk_documents_visibility
        CHECK (visibility IS NULL OR visibility IN ('public', 'private'));

ALTER TABLE context69.library_folders
    ADD COLUMN IF NOT EXISTS group_id BIGINT REFERENCES context69.groups(id) ON DELETE RESTRICT,
    ADD COLUMN IF NOT EXISTS project_id BIGINT REFERENCES context69.projects(id) ON DELETE RESTRICT,
    ADD COLUMN IF NOT EXISTS visibility TEXT,
    ADD CONSTRAINT chk_library_folders_visibility
        CHECK (visibility IS NULL OR visibility IN ('public', 'private'));

ALTER TABLE context69.library_files
    ADD COLUMN IF NOT EXISTS group_id BIGINT REFERENCES context69.groups(id) ON DELETE RESTRICT,
    ADD COLUMN IF NOT EXISTS project_id BIGINT REFERENCES context69.projects(id) ON DELETE RESTRICT,
    ADD COLUMN IF NOT EXISTS visibility TEXT,
    ADD CONSTRAINT chk_library_files_visibility
        CHECK (visibility IS NULL OR visibility IN ('public', 'private'));

ALTER TABLE context69.library_ingest_jobs
    ADD COLUMN IF NOT EXISTS group_id BIGINT REFERENCES context69.groups(id) ON DELETE RESTRICT,
    ADD COLUMN IF NOT EXISTS project_id BIGINT REFERENCES context69.projects(id) ON DELETE RESTRICT,
    ADD COLUMN IF NOT EXISTS visibility TEXT,
    ADD CONSTRAINT chk_library_ingest_jobs_visibility
        CHECK (visibility IS NULL OR visibility IN ('public', 'private'));

ALTER TABLE context69.library_file_documents
    ADD COLUMN IF NOT EXISTS group_id BIGINT REFERENCES context69.groups(id) ON DELETE RESTRICT,
    ADD COLUMN IF NOT EXISTS project_id BIGINT REFERENCES context69.projects(id) ON DELETE RESTRICT,
    ADD COLUMN IF NOT EXISTS visibility TEXT,
    ADD CONSTRAINT chk_library_file_documents_visibility
        CHECK (visibility IS NULL OR visibility IN ('public', 'private'));

WITH default_scope AS (
    SELECT g.id AS group_id, p.id AS project_id
    FROM context69.groups g
    JOIN context69.projects p ON p.group_id = g.id
    WHERE g.group_key = 'public'
      AND p.project_key = 'default-public'
)
UPDATE context69.source_configs sc
SET group_id = ds.group_id,
    project_id = ds.project_id,
    visibility = COALESCE(sc.visibility, 'public')
FROM default_scope ds
WHERE sc.group_id IS NULL
   OR sc.project_id IS NULL
   OR sc.visibility IS NULL;

WITH default_scope AS (
    SELECT g.id AS group_id, p.id AS project_id
    FROM context69.groups g
    JOIN context69.projects p ON p.group_id = g.id
    WHERE g.group_key = 'public'
      AND p.project_key = 'default-public'
)
UPDATE context69.source_checkpoints cp
SET group_id = ds.group_id,
    project_id = ds.project_id,
    visibility = COALESCE(cp.visibility, 'public')
FROM default_scope ds
WHERE cp.group_id IS NULL
   OR cp.project_id IS NULL
   OR cp.visibility IS NULL;

WITH default_scope AS (
    SELECT g.id AS group_id, p.id AS project_id
    FROM context69.groups g
    JOIN context69.projects p ON p.group_id = g.id
    WHERE g.group_key = 'public'
      AND p.project_key = 'default-public'
)
UPDATE context69.sync_runs sr
SET group_id = ds.group_id,
    project_id = ds.project_id,
    visibility = COALESCE(sr.visibility, 'public')
FROM default_scope ds
WHERE sr.group_id IS NULL
   OR sr.project_id IS NULL
   OR sr.visibility IS NULL;

WITH default_scope AS (
    SELECT g.id AS group_id, p.id AS project_id
    FROM context69.groups g
    JOIN context69.projects p ON p.group_id = g.id
    WHERE g.group_key = 'public'
      AND p.project_key = 'default-public'
)
UPDATE context69.documents d
SET group_id = ds.group_id,
    project_id = ds.project_id,
    visibility = COALESCE(d.visibility, 'public')
FROM default_scope ds
WHERE d.group_id IS NULL
   OR d.project_id IS NULL
   OR d.visibility IS NULL;

WITH default_scope AS (
    SELECT g.id AS group_id, p.id AS project_id
    FROM context69.groups g
    JOIN context69.projects p ON p.group_id = g.id
    WHERE g.group_key = 'public'
      AND p.project_key = 'default-public'
)
UPDATE context69.library_folders lf
SET group_id = ds.group_id,
    project_id = ds.project_id,
    visibility = COALESCE(lf.visibility, 'public')
FROM default_scope ds
WHERE lf.group_id IS NULL
   OR lf.project_id IS NULL
   OR lf.visibility IS NULL;

WITH default_scope AS (
    SELECT g.id AS group_id, p.id AS project_id
    FROM context69.groups g
    JOIN context69.projects p ON p.group_id = g.id
    WHERE g.group_key = 'public'
      AND p.project_key = 'default-public'
)
UPDATE context69.library_files lf
SET group_id = ds.group_id,
    project_id = ds.project_id,
    visibility = COALESCE(lf.visibility, 'public')
FROM default_scope ds
WHERE lf.group_id IS NULL
   OR lf.project_id IS NULL
   OR lf.visibility IS NULL;

WITH default_scope AS (
    SELECT g.id AS group_id, p.id AS project_id
    FROM context69.groups g
    JOIN context69.projects p ON p.group_id = g.id
    WHERE g.group_key = 'public'
      AND p.project_key = 'default-public'
)
UPDATE context69.library_ingest_jobs lj
SET group_id = ds.group_id,
    project_id = ds.project_id,
    visibility = COALESCE(lj.visibility, 'public')
FROM default_scope ds
WHERE lj.group_id IS NULL
   OR lj.project_id IS NULL
   OR lj.visibility IS NULL;

WITH default_scope AS (
    SELECT g.id AS group_id, p.id AS project_id
    FROM context69.groups g
    JOIN context69.projects p ON p.group_id = g.id
    WHERE g.group_key = 'public'
      AND p.project_key = 'default-public'
)
UPDATE context69.library_file_documents lfd
SET group_id = ds.group_id,
    project_id = ds.project_id,
    visibility = COALESCE(lfd.visibility, 'public')
FROM default_scope ds
WHERE lfd.group_id IS NULL
   OR lfd.project_id IS NULL
   OR lfd.visibility IS NULL;

ALTER TABLE context69.source_configs
    ALTER COLUMN group_id SET NOT NULL,
    ALTER COLUMN project_id SET NOT NULL,
    ALTER COLUMN visibility SET NOT NULL;

ALTER TABLE context69.source_checkpoints
    ALTER COLUMN group_id SET NOT NULL,
    ALTER COLUMN project_id SET NOT NULL,
    ALTER COLUMN visibility SET NOT NULL;

ALTER TABLE context69.sync_runs
    ALTER COLUMN group_id SET NOT NULL,
    ALTER COLUMN project_id SET NOT NULL,
    ALTER COLUMN visibility SET NOT NULL;

ALTER TABLE context69.documents
    ALTER COLUMN group_id SET NOT NULL,
    ALTER COLUMN project_id SET NOT NULL,
    ALTER COLUMN visibility SET NOT NULL;

ALTER TABLE context69.library_folders
    ALTER COLUMN group_id SET NOT NULL,
    ALTER COLUMN project_id SET NOT NULL,
    ALTER COLUMN visibility SET NOT NULL;

ALTER TABLE context69.library_files
    ALTER COLUMN group_id SET NOT NULL,
    ALTER COLUMN project_id SET NOT NULL,
    ALTER COLUMN visibility SET NOT NULL;

ALTER TABLE context69.library_ingest_jobs
    ALTER COLUMN group_id SET NOT NULL,
    ALTER COLUMN project_id SET NOT NULL,
    ALTER COLUMN visibility SET NOT NULL;

ALTER TABLE context69.library_file_documents
    ALTER COLUMN group_id SET NOT NULL,
    ALTER COLUMN project_id SET NOT NULL,
    ALTER COLUMN visibility SET NOT NULL;

CREATE INDEX IF NOT EXISTS idx_source_configs_project_id
    ON context69.source_configs (project_id, source_key);

CREATE INDEX IF NOT EXISTS idx_source_checkpoints_project_id
    ON context69.source_checkpoints (project_id, source_key);

CREATE INDEX IF NOT EXISTS idx_sync_runs_project_started_at
    ON context69.sync_runs (project_id, source_key, started_at DESC);

CREATE INDEX IF NOT EXISTS idx_documents_project_published_at
    ON context69.documents (project_id, published_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_documents_visibility_project_id
    ON context69.documents (visibility, project_id, id DESC);

CREATE INDEX IF NOT EXISTS idx_library_folders_project_parent_name
    ON context69.library_folders (project_id, parent_id, name);

CREATE INDEX IF NOT EXISTS idx_library_files_project_folder_filename
    ON context69.library_files (project_id, folder_id, filename);

CREATE INDEX IF NOT EXISTS idx_library_ingest_jobs_project_file_id
    ON context69.library_ingest_jobs (project_id, file_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_library_file_documents_project_document_id
    ON context69.library_file_documents (project_id, document_id);
