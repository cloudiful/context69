CREATE TABLE IF NOT EXISTS context69.personal_access_tokens (
    id UUID PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES context69.users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    display_prefix TEXT NOT NULL,
    scopes TEXT[] NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    last_used_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (btrim(name) <> ''),
    CHECK (btrim(token_hash) <> ''),
    CHECK (btrim(display_prefix) <> ''),
    CHECK (cardinality(scopes) > 0)
);

CREATE INDEX IF NOT EXISTS idx_personal_access_tokens_user_id
    ON context69.personal_access_tokens (user_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_personal_access_tokens_active
    ON context69.personal_access_tokens (user_id, revoked_at, expires_at);
