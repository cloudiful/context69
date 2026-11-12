-- Delete at most one fixed batch of provably disposable old original registry
-- entries while the old `document_chunks` rows still exist.
--
-- Eligibility is proven per id, never by whole-library array membership:
--
-- * a switched entry whose nonempty staged and retired ids are all present in
--   `document_chunks` is an old original lifecycle whose ids the rebuild
--   replaces. Translation ids never live in `document_chunks`, so translation
--   entries are skipped instead of guessed away;
-- * an id-less switched reservation is the exact shape the original rebuild
--   reservation producer writes (`admit_reserved_entry` with empty arrays,
--   NULL hashes, a fingerprint and a collection). Ordinary admissions always
--   stage at least one id and metadata-only tombstones reserve zero ids.
--
-- The proof is applied before `LIMIT`, so the batch is a bounded page of
-- eligible rows and remaining eligible rows are the progress. Business
-- tombstones and unprovable entries stay untouched.
WITH candidates AS (
    SELECT e.run_id
    FROM context69.search_registry_entries e
    WHERE e.phase = 'switched'
      AND (
          (
              cardinality(e.staged_ids) + cardinality(e.retired_ids) > 0
              AND NOT EXISTS (
                  SELECT 1
                  FROM unnest(e.staged_ids || e.retired_ids) AS candidate(id)
                  WHERE NOT EXISTS (
                      SELECT 1
                      FROM context69.document_chunks c
                      WHERE c.id = candidate.id
                  )
              )
          )
          OR (
              cardinality(e.staged_ids) = 0
              AND cardinality(e.retired_ids) = 0
              AND cardinality(e.document_ids) = 0
              AND e.reserved_ids > 0
              AND e.baseline_record_hash IS NULL
              AND e.staged_record_hash IS NULL
              AND e.embedding_fingerprint IS NOT NULL
              AND e.collection_name IS NOT NULL
          )
      )
    ORDER BY e.created_at, e.run_id
    LIMIT $1
)
DELETE FROM context69.search_registry_entries e
USING candidates c
WHERE e.run_id = c.run_id
