-- Persist the final document id reserved for one new section of a run.
--
-- The caller holds the run row and has already read the key on the same
-- transaction, so a conflicting insert is a broken run identity and must fail
-- instead of silently replacing the earlier reservation.
INSERT INTO context69.library_ingest_run_sections
    (run_id, section_key, file_id, section_external_id, document_id)
VALUES ($1, $2, $3, $4, $5)
RETURNING document_id
