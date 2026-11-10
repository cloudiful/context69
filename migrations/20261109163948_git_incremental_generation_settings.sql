-- Persisted source selection/refresh settings plus the identity of the content
-- that produced the active generation (issue #28 phase P2).
--
-- The source row owns the user-editable include/exclude patterns, the
-- independent optional interval and webhook switch, the enabled flag, the
-- retention count, and the skip summary of the last completed index. The
-- generation identity records which pipeline version and configuration
-- fingerprint produced the currently indexed snapshot, so a repeated sync can
-- prove a snapshot is unchanged instead of rebuilding it.
--
-- Everything is additive: no applied migration is edited and no existing
-- column is reinterpreted. `refresh_policy` stays for the acquisition phase it
-- was written for; the independent interval/webhook pair is the settings the
-- incremental refresh behavior reads.

ALTER TABLE context69.git_repository_sources
    ADD COLUMN include_patterns TEXT[] NOT NULL DEFAULT '{}',
    ADD COLUMN exclude_patterns TEXT[] NOT NULL DEFAULT '{}',
    ADD COLUMN interval_seconds BIGINT,
    ADD COLUMN webhook_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN enabled BOOLEAN NOT NULL DEFAULT TRUE,
    ADD COLUMN retention_count BIGINT NOT NULL DEFAULT 2,
    ADD COLUMN last_skip_summary JSONB NOT NULL DEFAULT '{}'::jsonb,
    ADD COLUMN indexed_pipeline_version BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN indexed_config_fingerprint TEXT NOT NULL DEFAULT '';

ALTER TABLE context69.git_repository_sources
    ADD CONSTRAINT chk_git_repository_sources_interval
        CHECK (interval_seconds IS NULL OR (interval_seconds BETWEEN 300 AND 2592000)),
    ADD CONSTRAINT chk_git_repository_sources_retention
        CHECK (retention_count BETWEEN 1 AND 50),
    ADD CONSTRAINT chk_git_repository_sources_patterns
        CHECK (cardinality(include_patterns) <= 128 AND cardinality(exclude_patterns) <= 128),
    ADD CONSTRAINT chk_git_repository_sources_skip_summary
        CHECK (jsonb_typeof(last_skip_summary) = 'object'),
    ADD CONSTRAINT chk_git_repository_sources_pipeline
        CHECK (indexed_pipeline_version >= 0);

CREATE INDEX IF NOT EXISTS idx_git_repository_sources_enabled
    ON context69.git_repository_sources (enabled, index_status, updated_at DESC);
