-- Lock the singleton Docling settings row so concurrent submitters serialize
-- their admission check. Returns zero rows when Docling is unconfigured;
-- callers fall back to the single-worker default (1).
SELECT max_inflight
FROM context69.docling_settings
WHERE singleton = TRUE
FOR UPDATE
