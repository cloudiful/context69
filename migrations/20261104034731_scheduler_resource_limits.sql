ALTER TABLE context69.runtime_scheduler_settings
    ADD COLUMN interactive_max_concurrency integer
        CHECK (interactive_max_concurrency BETWEEN 1 AND 1024),
    ADD COLUMN bulk_max_concurrency integer
        CHECK (bulk_max_concurrency BETWEEN 1 AND 1024),
    ADD COLUMN maintenance_max_concurrency integer
        CHECK (maintenance_max_concurrency BETWEEN 1 AND 1024),
    ADD COLUMN ingest_max_concurrency integer
        CHECK (ingest_max_concurrency BETWEEN 1 AND 1024);
