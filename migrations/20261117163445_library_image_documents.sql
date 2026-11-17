-- Parent/occurrence links between a stored image file and the document it
-- belongs to. This is additive: no existing table, row or constraint changes,
-- and no foreign key is introduced. File, document and group cleanup stays an
-- explicit service responsibility.
CREATE TABLE IF NOT EXISTS context69.library_image_documents (
    file_id UUID NOT NULL,
    document_id BIGINT NOT NULL,
    group_id BIGINT NOT NULL,
    role TEXT NOT NULL,
    ordinal INTEGER NOT NULL DEFAULT 0,
    caption TEXT,
    context_text TEXT,
    source_uri TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (file_id, document_id, role, ordinal),
    CONSTRAINT chk_library_image_documents_role
        CHECK (role IN ('cover', 'body')),
    CONSTRAINT chk_library_image_documents_ordinal
        CHECK (ordinal >= 0),
    CONSTRAINT chk_library_image_documents_caption
        CHECK (caption IS NULL OR char_length(caption) <= 1024),
    CONSTRAINT chk_library_image_documents_context
        CHECK (context_text IS NULL OR char_length(context_text) <= 8192)
);

-- One occurrence slot of a document holds one image, whichever file stores it.
CREATE UNIQUE INDEX IF NOT EXISTS uq_library_image_documents_occurrence
    ON context69.library_image_documents (document_id, role, ordinal);

CREATE INDEX IF NOT EXISTS idx_library_image_documents_group_document
    ON context69.library_image_documents (group_id, document_id);

CREATE INDEX IF NOT EXISTS idx_library_image_documents_file
    ON context69.library_image_documents (file_id);
