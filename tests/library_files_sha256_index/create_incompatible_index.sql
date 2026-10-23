-- A same-name index that is not the expected lookup index: it carries only
-- the equality pair, so it cannot serve the deterministic lookup order.
CREATE INDEX idx_library_files_group_sha256
    ON context69.library_files (group_id, sha256)
