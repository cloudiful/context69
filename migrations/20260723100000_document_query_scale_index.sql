CREATE INDEX IF NOT EXISTS idx_documents_group_source_published_id
    ON context69.documents (group_id, source_key, published_at DESC, id ASC);
