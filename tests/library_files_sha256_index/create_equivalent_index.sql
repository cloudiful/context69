-- The exact index an operator pre-creates CONCURRENTLY before a production
-- upgrade: a plain non-unique btree over the dedup equality pair followed by
-- the order the lookup asks for.
--
-- The index name is never schema-qualified: PostgreSQL always creates an
-- index in the schema of its table, so this lands in `context69` next to
-- `context69.library_files`.
CREATE INDEX idx_library_files_group_sha256
    ON context69.library_files (group_id, sha256, created_at, id)
