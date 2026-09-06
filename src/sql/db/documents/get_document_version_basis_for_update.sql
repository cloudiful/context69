-- get_document_version_basis_for_update.sql (issue 139, phase 1)
--
-- Lock the parent document row for a library business-fields update so the
-- caller can compare the stored record_hash with the incoming payload hash
-- inside the same transaction. Title/summary are returned as the current
-- document fields for the version snapshot; source_uri/published_at/
-- metadata_json/record_hash for the snapshot come from the incoming payload
-- because those are the values being written by the business-fields update.
SELECT record_hash, title, summary
FROM context69.documents
WHERE id = $1
FOR UPDATE
