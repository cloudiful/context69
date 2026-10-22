-- Remove the applied dedup lookup index so a fixture can start from a schema
-- that carries no same-name object.
DROP INDEX IF EXISTS context69.idx_library_files_group_sha256
