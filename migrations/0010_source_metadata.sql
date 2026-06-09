ALTER TABLE context69.source_configs
    ADD COLUMN IF NOT EXISTS display_name TEXT,
    ADD COLUMN IF NOT EXISTS description TEXT,
    ADD COLUMN IF NOT EXISTS example_queries JSONB NOT NULL DEFAULT '[]'::jsonb;
