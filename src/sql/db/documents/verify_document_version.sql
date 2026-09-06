-- verify_document_version.sql (issue 139, phase 4)
--
-- Read-only existence check that the matching `document_versions` row is
-- present for (`document_id`, `record_hash`). Used to detect already-fixed
-- documents and to verify the INSERT before committing each per-document
-- backfill transaction. SELECT-only; never mutates.
SELECT EXISTS(
    SELECT 1
    FROM context69.document_versions
    WHERE document_id = $1
      AND record_hash = $2
)
