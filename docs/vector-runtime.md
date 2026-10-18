# Vector runtime

Context69 needs a vector runtime to index and search library content: an
embedding provider that produces the vectors and a Qdrant collection that
stores them with the collection identity and dimensions the instance is
configured for. The runtime is required. A running instance without it keeps
management and the API reachable but must not claim that library processing or
search is ready.

## Health endpoints

| Path | Question it answers | Depends on |
| --- | --- | --- |
| `/livez` | Is this process running? | Nothing. No database, vector, or model call. |
| `/readyz` | Can this instance serve core work now? | Database, the current vector runtime generation, and the library processing verdict. |
| `/healthz` | Readiness. Same check as `/readyz`. | Same as `/readyz`. |

`/readyz` and `/healthz` answer `200` only when the database answers, the
vector runtime completes a bounded read-only probe, and the library processing
verdict is ready. They answer `503` otherwise, with the fields that describe
what actually failed:

- `status` is `ok` on `200` and `degraded` on `503`;
- `db_ok` and `qdrant_ok` report the individual results on `503`;
- `library_processing_ready` reports the processing verdict, so a `503` stays
  truthful even when the database and Qdrant are individually healthy;
- `indexed_chunks` is present only on `200`, because it comes from the probe
  that succeeded.

`/healthz` keeps its existing response shape. `/livez` reports process state
only: a `200` there is not proof that vector work can proceed.

The probe is bounded and read-only. It reads the runtime generation current
when the request starts and confirms the same generation is still current when
it completes, so a settings change or a rebuild that starts mid-probe yields
`503` instead of a stale `200`.

## Startup and recovery

A Qdrant that is slow to start, restoring, or briefly unreachable during
startup does not require a restart of Context69. Startup retries the vector
runtime within a bounded budget (30 seconds total, at most 5 seconds per
attempt, exponential backoff). If the budget expires, the process keeps serving
management traffic and readiness stays `503`.

One recovery supervisor then owns the runtime for the process lifetime:

- while no usable client is published it retries with capped backoff;
- once a client is published it probes every 10 seconds with a 5 second bound;
- a failed scheduled probe marks the instance not ready and keeps the published
  client for a later probe, while a successful probe restores the same
  generation;
- a transient request failure alone does not clear readiness;
- an authentication, dimension, invalid-configuration, or known-missing
  collection failure pauses rapid retries until the configuration changes,
  while logging a sanitized warning.

Recovery is read-only against an existing collection: it checks existence,
identity, dimensions, and a count, and never creates, deletes, recreates, or
re-indexes one. A collection is created only when the installation has no prior
vector state, and a concurrent create that already exists is re-read and
validated. Rebuilding an existing collection remains the explicit rebuild
operation with its own safety gate.

Every vector consumer reads one coherent provider and index pair from the
shared runtime, so a recovered runtime becomes usable by library ingestion,
source sync, translation, search, the document store, and rebuilds without a
restart. A rebuild in progress keeps writers and readers stopped together until
it completes, and a completion from a superseded generation cannot reopen
readiness.

## Dependency waits

When the vector runtime cannot serve work, ingestion and source sync wait under
the existing dependency protocol with the `qdrant` dependency key and the
existing next-attempt backoff. They do not consume a normal failure attempt and
do not change task status or counters. Ordinary invalid input, schema, and
permanent configuration errors keep their original failure behaviour.

## Not configured

Without runtime settings the API and management UI stay reachable, `/livez`
answers `200`, and `/readyz` plus `/healthz` answer `503`. The dependency gates
report the missing configuration, and nothing retries a model or network call
until the runtime is configured.

## Diagnostics

Recovery logs identify the operation category, the generation, and the failure
class. They never include credentials, credentials embedded in endpoint URLs,
or full remote payloads.
