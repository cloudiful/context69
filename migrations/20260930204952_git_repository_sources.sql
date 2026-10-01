-- Provider-neutral Git repository and connection persistence (issue #681 phase 2).
--
-- Phase 2 persists Git repository sources, provider connection metadata,
-- webhook ownership, and webhook delivery idempotency so later indexing and
-- webhook phases have a stable foundation. No provider call, clone, or route
-- exists yet.
--
-- Secret material never lands in these rows: read credentials and webhook
-- signing secrets are referenced through
-- context69.internal_secrets(key), the existing secret store.
--
-- `git_repository_sources` holds one row per (canonical_url, target_ref) and
-- carries the independent version, refresh, and index-profile policies plus
-- the commit checkpoint later incremental sync diffs from: `indexed_commit_sha`
-- is the last fully indexed commit, `target_commit_sha` the newest observed
-- one, and `checkpoint_updated_at` records when that pair last moved.
--
-- `git_webhook_deliveries` is keyed by the provider delivery id, so a
-- redelivered webhook inserts nothing and cannot double-enqueue work.

CREATE TABLE IF NOT EXISTS context69.git_provider_connections (
    connection_key TEXT PRIMARY KEY,
    provider_kind TEXT NOT NULL,
    connection_mode TEXT NOT NULL,
    display_name TEXT NOT NULL,
    base_url TEXT NOT NULL,
    credential_secret_key TEXT REFERENCES context69.internal_secrets(key) ON DELETE SET NULL,
    webhook_secret_key TEXT REFERENCES context69.internal_secrets(key) ON DELETE SET NULL,
    disabled_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT chk_git_provider_connections_key
        CHECK (btrim(connection_key) <> ''),
    CONSTRAINT chk_git_provider_connections_provider
        CHECK (provider_kind IN ('github', 'forgejo', 'gitlab', 'generic')),
    CONSTRAINT chk_git_provider_connections_mode
        CHECK (connection_mode IN ('public', 'installation', 'token')),
    CONSTRAINT chk_git_provider_connections_display
        CHECK (btrim(display_name) <> ''),
    CONSTRAINT chk_git_provider_connections_base_url
        CHECK (btrim(base_url) <> '')
);

CREATE TABLE IF NOT EXISTS context69.git_repository_sources (
    repository_key UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    connection_key TEXT REFERENCES context69.git_provider_connections(connection_key) ON DELETE SET NULL,
    provider_kind TEXT NOT NULL,
    canonical_url TEXT NOT NULL,
    repository_owner TEXT NOT NULL,
    repository_name TEXT NOT NULL,
    default_branch TEXT NOT NULL,
    target_ref TEXT NOT NULL,
    target_commit_sha TEXT,
    indexed_commit_sha TEXT,
    index_profile TEXT NOT NULL,
    refresh_policy TEXT NOT NULL,
    index_status TEXT NOT NULL DEFAULT 'pending',
    last_indexed_at TIMESTAMPTZ,
    checkpoint_updated_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT uq_git_repository_sources_ref
        UNIQUE (canonical_url, target_ref),
    CONSTRAINT chk_git_repository_sources_provider
        CHECK (provider_kind IN ('github', 'forgejo', 'gitlab', 'generic')),
    CONSTRAINT chk_git_repository_sources_url
        CHECK (btrim(canonical_url) <> ''),
    CONSTRAINT chk_git_repository_sources_owner
        CHECK (btrim(repository_owner) <> ''),
    CONSTRAINT chk_git_repository_sources_name
        CHECK (btrim(repository_name) <> ''),
    CONSTRAINT chk_git_repository_sources_branch
        CHECK (btrim(default_branch) <> ''),
    CONSTRAINT chk_git_repository_sources_ref
        CHECK (btrim(target_ref) <> ''),
    CONSTRAINT chk_git_repository_sources_profile
        CHECK (index_profile IN ('lexical', 'hybrid', 'full_semantic')),
    CONSTRAINT chk_git_repository_sources_refresh
        CHECK (refresh_policy IN ('manual', 'webhook', 'reconcile')),
    CONSTRAINT chk_git_repository_sources_status
        CHECK (index_status IN ('pending', 'indexing', 'ready', 'stale', 'failed', 'disabled'))
);

CREATE INDEX IF NOT EXISTS idx_git_repository_sources_connection
    ON context69.git_repository_sources (connection_key);

CREATE INDEX IF NOT EXISTS idx_git_repository_sources_status
    ON context69.git_repository_sources (index_status, updated_at DESC);

CREATE TABLE IF NOT EXISTS context69.git_webhook_registrations (
    repository_key UUID PRIMARY KEY
        REFERENCES context69.git_repository_sources(repository_key) ON DELETE CASCADE,
    provider_kind TEXT NOT NULL,
    external_hook_id TEXT NOT NULL,
    ownership TEXT NOT NULL,
    active BOOLEAN NOT NULL DEFAULT TRUE,
    signing_secret_key TEXT REFERENCES context69.internal_secrets(key) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT chk_git_webhook_registrations_hook
        CHECK (btrim(external_hook_id) <> ''),
    CONSTRAINT chk_git_webhook_registrations_ownership
        CHECK (ownership IN ('integration', 'external', 'unknown'))
);

CREATE UNIQUE INDEX IF NOT EXISTS uq_git_webhook_registrations_hook
    ON context69.git_webhook_registrations (provider_kind, external_hook_id);

CREATE TABLE IF NOT EXISTS context69.git_webhook_deliveries (
    delivery_id TEXT PRIMARY KEY,
    provider_kind TEXT NOT NULL,
    repository_key UUID REFERENCES context69.git_repository_sources(repository_key) ON DELETE SET NULL,
    status TEXT NOT NULL DEFAULT 'received',
    target_commit_sha TEXT,
    received_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    processed_at TIMESTAMPTZ,
    CONSTRAINT chk_git_webhook_deliveries_id
        CHECK (btrim(delivery_id) <> ''),
    CONSTRAINT chk_git_webhook_deliveries_status
        CHECK (status IN ('received', 'queued', 'ignored', 'failed'))
);

CREATE INDEX IF NOT EXISTS idx_git_webhook_deliveries_repository
    ON context69.git_webhook_deliveries (repository_key, received_at DESC);

CREATE INDEX IF NOT EXISTS idx_git_webhook_deliveries_pending
    ON context69.git_webhook_deliveries (status, received_at)
    WHERE status IN ('received', 'queued');
