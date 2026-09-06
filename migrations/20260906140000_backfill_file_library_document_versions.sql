-- Backfill missing `file_library` document version snapshots (issue #139).
--
-- Single-deployment SQLx data migration after 20260906130000. Runs once
-- inside the SQLx migration-runner transaction; never modify an applied
-- migration, only add new ones.
--
-- Scope is hardcoded to `d.source_key = 'file_library'` and only rows
-- whose current `documents.record_hash` has no matching
-- `document_versions` row (`NOT EXISTS` on `(document_id, record_hash)`).
-- The current `documents.record_hash` is the source of truth: this
-- statement restores the required matching version snapshot from the
-- current document row plus its current ordered chunks.
--
-- Body reconstruction uses
-- `string_agg(c.chunk_text, E'\n' ORDER BY c.chunk_index)` so chunk order
-- matches the application join on `"\n"`.
--
-- `HAVING` rejects unrepairable shapes without application code:
--   * zero chunks (`COUNT(*) > 0`; an inner join already excludes them,
--     the guard stays explicit),
--   * blank aggregate body (whitespace-only aggregate is rejected),
--   * duplicate indexes (`COUNT(*) = COUNT(DISTINCT c.chunk_index)`),
--   * non-zero start (`MIN(c.chunk_index) = 0`),
--   * gaps (`COUNT(*) = MAX - MIN + 1` with `BIGINT` casts so the
--     `COUNT` (`BIGINT`) versus `MIN`/`MAX` (`INTEGER`) comparison is
--     valid PostgreSQL typing).
--
-- Snapshot fields are the current document values: `document_id`,
-- `record_hash`, `title`, `summary`, aggregate body, `source_uri`,
-- `published_at`, and `metadata_json`. `ON CONFLICT (document_id,
-- record_hash) DO NOTHING` keeps reruns and concurrent restores
-- idempotent and never overwrites an existing version row.
--
-- INSERT-only into `context69.document_versions`. Never updates, deletes,
-- or inserts `documents`, `document_chunks`, tasks, attempts, jobs, or
-- existing version rows. No application URL, secret, task retry, or
-- `pgcrypto` dependency.

INSERT INTO context69.document_versions (
    document_id,
    record_hash,
    title,
    summary,
    body_text,
    source_uri,
    published_at,
    metadata_json
)
SELECT
    d.id,
    d.record_hash,
    d.title,
    d.summary,
    string_agg(c.chunk_text, E'\n' ORDER BY c.chunk_index),
    d.source_uri,
    d.published_at,
    d.metadata_json
FROM context69.documents d
JOIN context69.document_chunks c ON c.document_id = d.id
WHERE d.source_key = 'file_library'
  AND NOT EXISTS (
      SELECT 1
      FROM context69.document_versions v
      WHERE v.document_id = d.id
        AND v.record_hash = d.record_hash
  )
GROUP BY
    d.id,
    d.record_hash,
    d.title,
    d.summary,
    d.source_uri,
    d.published_at,
    d.metadata_json
HAVING COUNT(*) > 0
   AND COUNT(*) = COUNT(DISTINCT c.chunk_index)
   AND MIN(c.chunk_index) = 0
   AND COUNT(*) = (MAX(c.chunk_index)::BIGINT - MIN(c.chunk_index)::BIGINT + 1)
   AND btrim(string_agg(c.chunk_text, E'\n' ORDER BY c.chunk_index), E' \t\n\r') <> ''
ON CONFLICT (document_id, record_hash) DO NOTHING;
