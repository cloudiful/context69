-- The library source policy a group persists for future uploads.
--
-- The setting is group-scoped, not deployment-global: a future upload reads it
-- from the owning group and passes the existing `SourcePolicy` explicitly.
-- Existing files keep the policy they were uploaded with; this column never
-- rewrites or backfills them. `retain` stays the default for every group.
ALTER TABLE context69.groups
    ADD COLUMN default_source_policy TEXT NOT NULL DEFAULT 'retain';

ALTER TABLE context69.groups
    ADD CONSTRAINT groups_default_source_policy_check
        CHECK (default_source_policy IN ('retain', 'release_after_processing'));
