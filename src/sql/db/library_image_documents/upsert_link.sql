-- Insert or converge the exact occurrence link inside the locked transaction.
--
-- $1 file_id, $2 document_id, $3 group_id, $4 role, $5 ordinal, $6 caption,
-- $7 context_text, $8 source_uri.
INSERT INTO context69.library_image_documents (
    file_id,
    document_id,
    group_id,
    role,
    ordinal,
    caption,
    context_text,
    source_uri
)
VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
ON CONFLICT (file_id, document_id, role, ordinal) DO UPDATE
SET group_id = EXCLUDED.group_id,
    caption = EXCLUDED.caption,
    context_text = EXCLUDED.context_text,
    source_uri = EXCLUDED.source_uri,
    updated_at = now()
