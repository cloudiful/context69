-- Group-owned Git repository connections plus durable index generations
-- (issue #681 work unit 3B1).
--
-- Additive only: no phase 2 table is rewritten, and no existing column is
-- reinterpreted. Phase 2 persisted Git sources, connections, webhook
-- ownership, and delivery idempotency without an owning group, without a
-- group-scoped identity, and without any record of which index generation is
-- serving a repository. This migration adds all three.
--
-- Ownership is mandatory. A provider connection and a repository source
-- belong to exactly one real group: there is no shared/synthetic owner and no
-- owner-less row. Group visibility is never stored on these tables; it is read
-- from context69.groups on every query, so a visibility change takes effect
-- immediately instead of being served from a stale snapshot.
--
-- Per-group repository/ref identity: the same canonical URL and ref may be
-- registered independently by two groups, and each group sees only its own
-- rows. Group-scoped reads and writes match on the owning group, so a group
-- can neither read nor overwrite another group's registration.
--
-- Secret material still lives in context69.internal_secrets; credential and
-- webhook signing columns keep referencing that store and stay nullable so
-- public sources need no credential.
--
-- Generations are metadata only (counts, provenance, pinned commit): no file,
-- blob, or chunk content is stored here. A generation is created 'building',
-- and completion writes its counts, marks it ready, supersedes the previously
-- active generation, and moves the repository's active-generation pointer in
-- one statement, so no window exists where a generation is ready but not
-- active or active but not ready.

-- Phase 2 registered no Git source or connection through any API, so these
-- tables hold no rows that could be attributed to a group. Refuse to guess
-- ownership instead of inventing a shared group.
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM context69.git_provider_connections)
       OR EXISTS (SELECT 1 FROM context69.git_repository_sources) THEN
        RAISE EXCEPTION
            'git provider connections and repository sources must be empty before group ownership is added';
    END IF;
END
$$;

-- Group ownership of repository sources, with per-group repository/ref
-- identity replacing the globally unique canonical URL and ref.
ALTER TABLE context69.git_repository_sources
    ADD COLUMN group_id BIGINT NOT NULL REFERENCES context69.groups(id) ON DELETE CASCADE;

ALTER TABLE context69.git_repository_sources
    DROP CONSTRAINT uq_git_repository_sources_ref;

ALTER TABLE context69.git_repository_sources
    ADD CONSTRAINT uq_git_repository_sources_group_ref
        UNIQUE (group_id, canonical_url, target_ref);

-- Group ownership of provider connections. The connection key becomes
-- group-scoped so two groups can register the same provider connection name
-- without one group taking over the other's row on upsert.
ALTER TABLE context69.git_provider_connections
    ADD COLUMN group_id BIGINT NOT NULL REFERENCES context69.groups(id) ON DELETE CASCADE;

-- Dropped first: the phase 2 reference targets the old single-column primary
-- key, which the group-scoped key below replaces.
ALTER TABLE context69.git_repository_sources
    DROP CONSTRAINT git_repository_sources_connection_key_fkey;

ALTER TABLE context69.git_provider_connections
    DROP CONSTRAINT git_provider_connections_pkey;

ALTER TABLE context69.git_provider_connections
    ADD CONSTRAINT git_provider_connections_pkey PRIMARY KEY (group_id, connection_key);

CREATE INDEX IF NOT EXISTS idx_git_provider_connections_group
    ON context69.git_provider_connections (group_id, connection_key);

-- The connection reference follows the group: a source can only point at a
-- connection owned by the same group. Detaching a deleted connection clears
-- the reference while the source keeps its own mandatory group.
ALTER TABLE context69.git_repository_sources
    ADD CONSTRAINT fk_git_repository_sources_connection
        FOREIGN KEY (group_id, connection_key)
        REFERENCES context69.git_provider_connections (group_id, connection_key)
        ON DELETE SET NULL (connection_key);

CREATE INDEX IF NOT EXISTS idx_git_repository_sources_group
    ON context69.git_repository_sources (group_id, created_at DESC, repository_key);

-- Durable index generations. One row per indexed snapshot of a repository
-- source, numbered per repository, carrying the pinned commit the generation
-- covers plus the coverage counts acquisition reported. Content lives in the
-- lexical/vector stores, not here.
CREATE TABLE IF NOT EXISTS context69.git_repository_generations (
    generation_key UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    repository_key UUID NOT NULL
        REFERENCES context69.git_repository_sources(repository_key) ON DELETE CASCADE,
    generation_number BIGINT NOT NULL,
    ref_name TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    index_profile TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'building',
    file_count BIGINT NOT NULL DEFAULT 0,
    excluded_file_count BIGINT NOT NULL DEFAULT 0,
    total_bytes BIGINT NOT NULL DEFAULT 0,
    error_code TEXT,
    started_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT uq_git_repository_generations_number
        UNIQUE (repository_key, generation_number),
    -- Lets the active-generation pointer reference a generation of its own
    -- repository, so a repository can never point at another repository's
    -- snapshot.
    CONSTRAINT uq_git_repository_generations_key
        UNIQUE (repository_key, generation_key),
    CONSTRAINT chk_git_repository_generations_number
        CHECK (generation_number > 0),
    CONSTRAINT chk_git_repository_generations_ref
        CHECK (btrim(ref_name) <> ''),
    CONSTRAINT chk_git_repository_generations_commit
        CHECK (btrim(commit_sha) <> ''),
    CONSTRAINT chk_git_repository_generations_profile
        CHECK (index_profile IN ('lexical', 'hybrid', 'full_semantic')),
    CONSTRAINT chk_git_repository_generations_status
        CHECK (status IN ('building', 'ready', 'failed', 'superseded')),
    CONSTRAINT chk_git_repository_generations_counts
        CHECK (file_count >= 0 AND excluded_file_count >= 0 AND total_bytes >= 0),
    -- A generation is building exactly while it is incomplete, so a completed
    -- generation can never claim to still be building or vice versa.
    CONSTRAINT chk_git_repository_generations_completion
        CHECK ((status = 'building') = (completed_at IS NULL))
);

CREATE INDEX IF NOT EXISTS idx_git_repository_generations_repository
    ON context69.git_repository_generations (repository_key, generation_number DESC);

CREATE INDEX IF NOT EXISTS idx_git_repository_generations_status
    ON context69.git_repository_generations (status, started_at DESC)
    WHERE status = 'building';

-- Which generation a repository source currently serves from. The pointer
-- exists only for a repository that has an activated generation, and the
-- composite foreign key confines it to that repository's own generations.
CREATE TABLE IF NOT EXISTS context69.git_repository_active_generations (
    repository_key UUID PRIMARY KEY
        REFERENCES context69.git_repository_sources(repository_key) ON DELETE CASCADE,
    generation_key UUID NOT NULL,
    activated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT fk_git_repository_active_generations_generation
        FOREIGN KEY (repository_key, generation_key)
        REFERENCES context69.git_repository_generations (repository_key, generation_key)
        ON DELETE CASCADE
);
