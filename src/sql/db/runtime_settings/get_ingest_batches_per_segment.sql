-- The configured number of ingest batches one task indexing segment commits.
-- A missing singleton row uses the application default of one.

SELECT ingest_batches_per_segment
FROM context69.runtime_scheduler_settings
WHERE singleton = TRUE
