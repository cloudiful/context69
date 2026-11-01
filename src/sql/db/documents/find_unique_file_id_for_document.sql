-- The distinct library files that link this document.
--
-- The unique-file publication gate needs to know whether the document belongs
-- to exactly one file across every section mapping. Two rows already prove the
-- document is ambiguous, so the reader stops there.
SELECT DISTINCT file_id
FROM context69.library_file_documents
WHERE document_id = $1
LIMIT 2
