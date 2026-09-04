-- Serialize Docling admission check-and-insert across processes.
--
-- The singleton `docling_settings` row lock covers the configured case, but
-- an unconfigured database has no row to lock. This transaction-scoped
-- advisory lock (constant key) ensures concurrent submitters still serialize
-- when the settings row is missing and the default limit applies.
SELECT pg_advisory_xact_lock(727335732)
