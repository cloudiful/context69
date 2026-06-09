CREATE TABLE IF NOT EXISTS context69.docling_settings (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE,
    base_url TEXT NOT NULL,
    timeout_secs BIGINT NOT NULL,
    poll_interval_secs BIGINT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (singleton),
    CHECK (btrim(base_url) <> ''),
    CHECK (timeout_secs > 0),
    CHECK (poll_interval_secs > 0)
);
