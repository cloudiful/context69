-- Whether the dedup lookup index name resolves in the owned schema.
SELECT to_regclass('context69.idx_library_files_group_sha256') IS NOT NULL AS present
