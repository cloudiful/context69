ALTER TABLE context69.runtime_scheduler_settings
    ADD COLUMN ingest_batches_per_segment integer NOT NULL DEFAULT 1
        CHECK (ingest_batches_per_segment BETWEEN 1 AND 64);
