-- A same-name object that is not an index at all. `CREATE INDEX IF NOT
-- EXISTS` would only emit a notice for it and skip the index, so the
-- migration's guard has to refuse it.
CREATE TABLE context69.idx_library_files_group_sha256 (id BIGINT)
