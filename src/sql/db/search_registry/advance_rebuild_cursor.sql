-- Advances the committed cursor of one owned rebuild page (issue 1212 P3).
--
-- The caller holds the item's write fence and only reaches this statement after
-- the page's vectors were written, so the recorded cursor never describes a
-- page that did not land. Everything that identifies the rebuild is part of the
-- predicate: the owner item, its grant, the captured fingerprint and the
-- physical collection. A rebuild that lost its ownership or was superseded by a
-- new vector identity matches no row and advances nothing.
UPDATE context69.search_registry_epoch
SET rebuild_cursor_document_id = $5::bigint,
    rebuild_cursor_chunk_index = $6::integer,
    rebuild_processed_chunks = $7::bigint,
    rebuild_total_chunks = $8::bigint,
    updated_at = now()
WHERE id = 1
  AND rebuild_state = 'running'
  AND rebuild_owner_item = $1
  AND rebuild_owner_token = $2
  AND rebuild_fingerprint = $3
  AND rebuild_collection = $4
  AND $7::bigint >= 0
  AND $8::bigint >= $7::bigint
RETURNING TRUE AS "updated!"
