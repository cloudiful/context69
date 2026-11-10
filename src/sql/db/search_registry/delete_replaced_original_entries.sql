-- Delete the registry entries whose live and retired ids are all replaced
-- original chunk ids, and the id-less reservations of an earlier rebuild.
-- Translation ids are never members of document_chunks, so translation entries
-- survive untouched. An ordinary admission always stages at least one id, so a
-- switched entry with neither staged nor retired ids can only be the counted
-- reservation a previous rebuild wrote for replaced original ids.
DELETE FROM context69.search_registry_entries e
WHERE e.phase = 'switched'
  AND (
      (
          cardinality(e.staged_ids) + cardinality(e.retired_ids) > 0
          AND e.staged_ids <@ ARRAY(SELECT id FROM context69.document_chunks)
          AND e.retired_ids <@ ARRAY(SELECT id FROM context69.document_chunks)
      )
      OR (
          cardinality(e.staged_ids) = 0
          AND cardinality(e.retired_ids) = 0
          AND e.reserved_ids > 0
      )
  )
