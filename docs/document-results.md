# Document Results

Document results are caller-produced, append-only JSON records associated with
one document and the exact source record hash they were produced from. Context69
stores them; it never executes a model, interprets a business schema, or
validates a payload beyond the envelope. A stock producer such as `stock.news`
or `stock.disclosure`, or any other bounded opaque producer, owns its schema and
publishing semantics.

This is the generic storage contract the Agent engine (the stock producer)
writes into. Context69 owns ingestion, indexing, and generic result storage.

## Envelope

A `CreateDocumentResultRequest` carries:

- `producer` — bounded opaque identifier, `1..=128` characters from
  `[A-Za-z0-9._:-]` (canonical producers are `stock.news` and `stock.disclosure`).
- `result_kind` — the producer's own kind, same alphabet and bound.
- `schema_ref` — the producer-owned schema reference, `1..=256` characters,
  nonblank.
- `source_record_hash` — the document `record_hash` the result was produced
  from, nonblank, at most 256 characters.
- `idempotency_key` — caller-chosen key, same alphabet and bound as `producer`.
- `result` — the opaque payload; it must be a JSON object.
- `provenance` (optional) — credential-free, typed, and limited to `model_name`,
  `workflow_version`, `run_id`, and `producer_config_hash` (each at most 256
  characters). No other field is accepted, and no credential belongs here.

The whole serialized request is bounded at 256 KiB. Unknown envelope or
provenance fields are rejected rather than ignored.

## Append and idempotency

`POST /v1/groups/by-path/{group_path}/documents/{document_id}/results` appends a
result and returns `200` for both an insert and an exact replay.

- Ownership is the authenticated `author_user_id`; the request has no author
  field.
- An exact retry — same document, author, producer, and idempotency key, with a
  semantically equal payload and provenance — returns the original immutable
  record. It never appends a second row.
- The same idempotency key with a changed envelope returns `409`.
- A new key whose `source_record_hash` no longer matches the document's current
  hash returns `409`. The replay check runs first, so an already-accepted
  delivery stays retryable after a later document update.
- Distinct runs use distinct keys and append distinct rows; two producers reusing
  the same key are distinct namespaces.

The append runs in one transaction that locks the document row, which serializes
against a concurrent document update, so the source-version check cannot race the
ingestion that changes it.

## Read

`GET /v1/groups/by-path/{group_path}/documents/{document_id}/results` lists one
document's results, newest first, and returns the document's current
`source_record_hash` so a caller can bind a subsequent append.

- `producer` and `result_kind` are optional filters; `after_id` continues from
  the last returned id; `limit` defaults to `50` and is bounded to `1..=100`.
- The read lists only the specified document and never modifies
  `documents.metadata_json`.

There is no generic result `DELETE` or `UPDATE` endpoint; records are immutable.

## Authorization

Both operations are group-scoped. The route group must be one the caller can
see, and the document must belong to that group; an invisible or non-member
document reads as `404`, so private document membership is never leaked. `POST`
additionally requires the `Maintainer` role (`403` otherwise), and any
group-scoped authorized reader can list. The append is scoped by the caller's
access scope, so a document outside it is not reachable.

## Storage and deletion

Results live in `context69.document_results`. The document reference uses the
existing document-owned data deletion semantics (`ON DELETE CASCADE`). The
author reference is `ON DELETE RESTRICT`: removing a user is blocked while
results they authored exist, so result provenance is never silently discarded.
Plan user deletion with that restriction in mind.
