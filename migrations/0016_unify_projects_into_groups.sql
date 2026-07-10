ALTER TABLE context69.groups
    ADD COLUMN IF NOT EXISTS full_path TEXT;

ALTER TABLE context69.groups
    DROP CONSTRAINT IF EXISTS groups_group_key_key;

CREATE TEMP TABLE project_group_map (
    project_id BIGINT PRIMARY KEY,
    group_id BIGINT NOT NULL
) ON COMMIT DROP;

INSERT INTO project_group_map (project_id, group_id)
SELECT p.id, g.id
FROM context69.projects p
JOIN context69.groups g ON g.id = p.group_id
WHERE g.group_key = 'public'
  AND p.project_key = 'default-public';

WITH inserted_groups AS (
    INSERT INTO context69.groups (
        parent_group_id,
        group_key,
        name,
        visibility,
        kind,
        owner_user_id,
        created_by_user_id,
        created_at,
        updated_at
    )
    SELECT
        p.group_id,
        p.project_key,
        p.name,
        p.visibility,
        'shared',
        NULL,
        p.created_by_user_id,
        p.created_at,
        p.updated_at
    FROM context69.projects p
    JOIN context69.groups g ON g.id = p.group_id
    WHERE NOT (g.group_key = 'public' AND p.project_key = 'default-public')
    RETURNING id, parent_group_id, group_key
)
INSERT INTO project_group_map (project_id, group_id)
SELECT p.id, inserted_groups.id
FROM context69.projects p
JOIN inserted_groups
  ON inserted_groups.parent_group_id = p.group_id
 AND inserted_groups.group_key = p.project_key;

UPDATE context69.source_configs sc
SET group_id = pgm.group_id
FROM project_group_map pgm
WHERE sc.project_id = pgm.project_id;

UPDATE context69.source_checkpoints sc
SET group_id = pgm.group_id
FROM project_group_map pgm
WHERE sc.project_id = pgm.project_id;

UPDATE context69.sync_runs sr
SET group_id = pgm.group_id
FROM project_group_map pgm
WHERE sr.project_id = pgm.project_id;

UPDATE context69.documents d
SET group_id = pgm.group_id
FROM project_group_map pgm
WHERE d.project_id = pgm.project_id;

UPDATE context69.library_folders lf
SET group_id = pgm.group_id
FROM project_group_map pgm
WHERE lf.project_id = pgm.project_id;

UPDATE context69.library_files lf
SET group_id = pgm.group_id
FROM project_group_map pgm
WHERE lf.project_id = pgm.project_id;

UPDATE context69.library_ingest_jobs lj
SET group_id = pgm.group_id
FROM project_group_map pgm
WHERE lj.project_id = pgm.project_id;

UPDATE context69.library_file_documents lfd
SET group_id = pgm.group_id
FROM project_group_map pgm
WHERE lfd.project_id = pgm.project_id;

INSERT INTO context69.group_memberships (group_id, user_id, role, created_at, updated_at)
SELECT
    pgm.group_id,
    pm.user_id,
    pm.role,
    pm.created_at,
    pm.updated_at
FROM context69.project_memberships pm
JOIN project_group_map pgm ON pgm.project_id = pm.project_id
ON CONFLICT (group_id, user_id) DO UPDATE
SET role = CASE
        WHEN group_memberships.role = 'owner' OR EXCLUDED.role = 'owner' THEN 'owner'
        WHEN group_memberships.role = 'maintainer' OR EXCLUDED.role = 'maintainer' THEN 'maintainer'
        ELSE 'viewer'
    END,
    updated_at = now();

WITH RECURSIVE group_paths AS (
    SELECT
        g.id,
        g.parent_group_id,
        g.group_key,
        g.group_key::text AS full_path
    FROM context69.groups g
    WHERE g.parent_group_id IS NULL

    UNION ALL

    SELECT
        child.id,
        child.parent_group_id,
        child.group_key,
        group_paths.full_path || '/' || child.group_key
    FROM context69.groups child
    JOIN group_paths ON child.parent_group_id = group_paths.id
)
UPDATE context69.groups g
SET full_path = group_paths.full_path
FROM group_paths
WHERE g.id = group_paths.id;

DROP INDEX IF EXISTS context69.uq_library_folders_project_parent_name;
DROP INDEX IF EXISTS context69.uq_library_files_project_folder_filename;
DROP INDEX IF EXISTS context69.uq_library_files_project_external_id;
DROP INDEX IF EXISTS context69.idx_source_configs_project_id;
DROP INDEX IF EXISTS context69.idx_source_checkpoints_project_id;
DROP INDEX IF EXISTS context69.idx_sync_runs_project_started_at;
DROP INDEX IF EXISTS context69.idx_documents_project_published_at;
DROP INDEX IF EXISTS context69.idx_documents_visibility_project_id;
DROP INDEX IF EXISTS context69.idx_library_folders_project_parent_name;
DROP INDEX IF EXISTS context69.idx_library_files_project_folder_filename;
DROP INDEX IF EXISTS context69.idx_library_ingest_jobs_project_created_at;
DROP INDEX IF EXISTS context69.idx_library_file_documents_project_id;

ALTER TABLE context69.documents
    DROP CONSTRAINT IF EXISTS documents_source_key_external_id_key;

ALTER TABLE context69.groups
    ALTER COLUMN full_path SET NOT NULL;

ALTER TABLE context69.groups
    ADD CONSTRAINT uq_groups_full_path UNIQUE (full_path);

CREATE UNIQUE INDEX IF NOT EXISTS uq_groups_parent_group_key
    ON context69.groups ((COALESCE(parent_group_id, 0)), group_key);

CREATE UNIQUE INDEX IF NOT EXISTS uq_documents_group_source_external_id
    ON context69.documents (group_id, source_key, external_id);

CREATE INDEX IF NOT EXISTS idx_documents_group_published_at
    ON context69.documents (group_id, published_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_documents_visibility_group_id
    ON context69.documents (visibility, group_id, id DESC);

CREATE UNIQUE INDEX IF NOT EXISTS uq_library_folders_group_parent_name
    ON context69.library_folders (group_id, (COALESCE(parent_id::text, '__root__')), name);

CREATE UNIQUE INDEX IF NOT EXISTS uq_library_files_group_folder_filename
    ON context69.library_files (group_id, (COALESCE(folder_id::text, '__root__')), filename);

CREATE UNIQUE INDEX IF NOT EXISTS uq_library_files_group_external_id
    ON context69.library_files (group_id, external_id)
    WHERE external_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_source_configs_group_id
    ON context69.source_configs (group_id, source_key);

CREATE INDEX IF NOT EXISTS idx_source_checkpoints_group_id
    ON context69.source_checkpoints (group_id, source_key);

CREATE INDEX IF NOT EXISTS idx_sync_runs_group_started_at
    ON context69.sync_runs (group_id, source_key, started_at DESC);

CREATE INDEX IF NOT EXISTS idx_library_folders_group_parent_name
    ON context69.library_folders (group_id, parent_id, name);

CREATE INDEX IF NOT EXISTS idx_library_files_group_folder_filename
    ON context69.library_files (group_id, folder_id, filename);

CREATE INDEX IF NOT EXISTS idx_library_ingest_jobs_group_created_at
    ON context69.library_ingest_jobs (group_id, file_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_library_file_documents_group_id
    ON context69.library_file_documents (group_id, document_id);

ALTER TABLE context69.source_configs
    DROP COLUMN IF EXISTS project_id;

ALTER TABLE context69.source_checkpoints
    DROP COLUMN IF EXISTS project_id;

ALTER TABLE context69.sync_runs
    DROP COLUMN IF EXISTS project_id;

ALTER TABLE context69.documents
    DROP COLUMN IF EXISTS project_id;

ALTER TABLE context69.library_folders
    DROP COLUMN IF EXISTS project_id;

ALTER TABLE context69.library_files
    DROP COLUMN IF EXISTS project_id;

ALTER TABLE context69.library_ingest_jobs
    DROP COLUMN IF EXISTS project_id;

ALTER TABLE context69.library_file_documents
    DROP COLUMN IF EXISTS project_id;

DROP TABLE IF EXISTS context69.project_memberships;
DROP TABLE IF EXISTS context69.projects;
