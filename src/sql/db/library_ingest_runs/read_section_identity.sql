-- The final document identity one durable run already reserved for a section.
--
-- The row is the authority of the section identity of that run: when it
-- exists, its document id and section identity are reused by every retry.
SELECT file_id, section_external_id, document_id
FROM context69.library_ingest_run_sections
WHERE run_id = $1
  AND section_key = $2
FOR UPDATE
