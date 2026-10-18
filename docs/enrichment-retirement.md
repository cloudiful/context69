# Enrichment retirement

[简体中文](enrichment-retirement.zh-CN.md)

Embedded enrichment was an opportunistic `extraction` object written into
`documents.metadata_json` by the retired model pipeline. Context69 retires it
during a normal version rollout: the standard SQLx migration imports the useful
payloads as generic document results, removes the embedded key, clears the
obsolete label and index definitions, and enqueues durable payload cleanup. The
rollout needs no operator step or extra credential.

## Migration

`20261017080707_retire_embedded_enrichment.sql` runs inside the normal
`Database::connect` migration step, in one transaction. A database without
affected data, such as a fresh deployment, passes the document guards without
needing a bootstrap admin, and still converges the obsolete task label and the
`extraction`/`extraction.*` metadata-index definitions.

A document is affected when `metadata_json` has the exact top-level key
`extraction`. Before any write, the migration aborts the whole transaction when
any affected document fails one of these guards:

- a current-hash `document_versions` snapshot exists and its metadata equals the
  document metadata with `extraction` removed, with the same title, summary, and
  source URI;
- that snapshot body is not blank;
- chunks exist and all carry the current `record_hash`;
- `extraction` is an object whose schemas are only `stock.news.v1` and
  `stock.disclosure.v1`, each holding an object that fits the 256 KiB result
  envelope;
- no composed library file or section metadata source would reintroduce the key;
- no ingestion task item or Docling remote job is active.

For every affected document and schema the migration appends one imported
result:

- producer `context69.enrichment-import`, result kind `imported-enrichment`,
  schema ref the original `stock.news.v1` or `stock.disclosure.v1` key;
- `source_record_hash` is the document's current hash;
- idempotency key `sqlx-retire:` plus a SHA-256 over the document id, schema
  ref, source hash, and canonical JSON payload;
- provenance `workflow_version=context69-enrichment-migration-20261017080707`
  and `run_id=context69-enrichment-retirement-sqlx`, with no model or
  credential fields;
- author is the lowest-id existing user with `is_admin` and no `disabled_at`.
  If affected data exists and no such administrator exists, the migration rolls
  back instead of creating an account or claiming an original author. An
  existing row under the same identity must match byte-for-byte; a different
  envelope aborts.

Then the same transaction removes only `documents.metadata_json -> 'extraction'`
and leaves the record hash, canonical snapshots, chunks, translations, vectors,
and task history intact. Independently of whether any document is affected, it
clears only the obsolete `failure_stage = 'extraction'` label on tasks and task
items and deletes `extraction`/`extraction.*` metadata-index definitions (their
values cascade). When something changed it bumps the search generation so cached
results miss once, and it inserts one outbox row per affected document.

## Payload convergence

External point payloads cannot join the database transaction. The migration
therefore writes `context69.document_metadata_payload_sync` with only the
document id and the fixed key `metadata_index.extraction`. A background worker
drains that outbox immediately at startup and then every 5 seconds:

- it reads the process-wide vector runtime on every pass and waits, without
  discarding jobs, until a ready pair exists. A late-starting or restored Qdrant
  is therefore picked up without a restart, and a rebuild or settings change
  fences new payload work until the replacement pair is ready;
- at most 16 jobs per pass, claimed with `FOR UPDATE SKIP LOCKED` and a 60-second
  lease, with each external operation bounded at 20 seconds;
- the attached collection is re-probed read-only, and every original point must
  exist with the exact `document_id` and `record_hash`; a missing original fails
  the attempt rather than being recreated;
- it deletes only the fixed nested key from those original points and from all
  translated chunk ids, including superseded translation generations. Absent
  historical translated points are counted no-ops;
- the generation is re-checked immediately before the delete and before
  completion, so a job never settles work against a superseded collection; it
  stays pending and retries;
- a failure records a sanitized reason and retries after 5 seconds, doubling to
  a 300-second cap, indefinitely. Nothing is discarded;
- completion is written only by the worker holding the current lease, so
  concurrent workers cannot both settle one job;
- the worker shares the recovery controller's shutdown token, so it stops with
  the application. An unconfigured vector runtime has no controller and starts
  no worker.

## Operator inspection

No manual step is required. To inspect convergence:

```sql
SELECT count(*) FILTER (WHERE completed_at IS NULL) AS pending,
       count(*) FILTER (WHERE completed_at IS NOT NULL) AS completed
FROM context69.document_metadata_payload_sync;

SELECT id, document_id, payload_key, attempts, next_attempt_at, last_error
FROM context69.document_metadata_payload_sync
WHERE completed_at IS NULL
ORDER BY next_attempt_at, id
LIMIT 50;
```

A pending row with an increasing `attempts` count and a `last_error` is retrying;
it never rolls back an already-migrated source generation. The migration and the
worker never read document text, log payload contents, call a model, re-embed
content, fetch an external source, or change a record hash.
