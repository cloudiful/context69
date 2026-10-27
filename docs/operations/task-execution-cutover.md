# Task execution cutover

[简体中文](task-execution-cutover.zh-CN.md)

The task center executes its items through the `cloudiful_tasks` library. Every
item carries a captured execution segment, so exactly one execution authority
owns it: a worker that no longer holds the segment cannot write, and a killed
worker's lease expires instead of leaving a half-finished row. An item that was
created before the library owned the execution has no segment at all.

That missing segment is what this cutover resolves. Two authorities must never
run the same item, so the task engine refuses to start while any item exists
without a segment, and the operator hands those items over explicitly.

## What owns what

The library owns the execution lease and the attempt lifecycle of an item: its
segment decides which worker may write, and a lease that is not renewed expires.
The domain row only mirrors that lease. Admission of the parent, and therefore
the eligibility of the item, stays a domain decision: linking a segment is not
admission, so an item whose parent may not take a slot stays queued. The item
payload, including its `indexing_checkpoint`, is written under the captured
grant, so a superseded generation cannot overwrite the checkpoint of its
successor.

A process that stops without running its asynchronous drop never releases the
lease it held, and nothing else releases it on that process's behalf: the
ordinary expiry path interrupts the library attempt and projects the failure
onto the item. The handover keeps every artifact, attempt row and payload it
finds. It links segments or closes interrupted items and never deletes history,
object references or an item's forensic trail.

## The startup latch

One readiness decision is taken per `TaskService` startup, before the first
dispatch and before any maintenance write:

- the `cloudiful_tasks` schema must exist, be owned by the library and be
  compatible; and
- no `queued`, `waiting` or `running` item may be left without an execution
  segment.

When the decision is blocked, the task engine starts nothing: no claim
dispatcher, no claim maintenance, no Docling recovery, adoption or cleanup, and
no task source-cleanup dispatcher. HTTP, manual task CRUD and the independent
services stay available. The readiness is logged once with its blocker code,
unlinked count and remediation hint under the default `context69=info` filter;
it is never re-evaluated, so a handover performed under a running process does
not open the engine beneath it. Restart the process after the handover.

`/healthz` and `/readyz` report the same process latch through the existing
`library_processing_ready` field and answer `503` while it is blocked. `/livez`
is unaffected: the process is still alive even when the engine is off.

Normal writable serving startup prepares the library schema through the
library's own migrate API after the application migrations, and `db_init` runs
the same preparation explicitly. A schema that is absent on a writable normal
start is created there; recovery admission, read-only connections and operator
connections never migrate or create it. The schema check fails closed: a
foreign or incompatible library schema fails startup visibly instead of being
adopted, and it blocks the engine rather than opening a second authority.

## The handover tool

`cargo run --bin task_execution_cutover -- <command> [flags]`

The tool reads its connection string only from `DATABASE_URL`; there is no
`.env` fallback and no URL argument. It connects without running migrations and
without upgrade admission, so it performs no DDL. Both the database name
configured in the URL and the name the server reports must equal the mandatory
`--expect-database`, and the library schema is validated before any read or
write.

`--workers-stopped` is mandatory for both handover commands. It is an operator
assertion, and the tool additionally refuses to write while either authority
still holds a live lease: a running item with a live mirror lease, or a library
attempt with a live lease over an item that still exists. Stop every task worker
before handing over.

Every command prints one line of JSON metadata and exits with `0` on success,
`2` for a usage error or a database target mismatch, and `3` for a database or
safety refusal. Credentials are stripped from every message, including driver
errors.

### `inventory`

Read-only, and the default first step. It lists a bounded page (at most 100
rows) of the items that block the engine, with the execution metadata that
decides what each row needs: item and parent ids, status, parent kind, ordinal,
whether the parent is retired, the execution generation and link, the retry
deadline and whether the item waits on a Docling dependency. Item payloads and
user data are not selected.

```
task_execution_cutover inventory --expect-database context69 --limit 100 --offset 0
```

### `adopt`

Links a fresh execution segment to explicitly named, unlinked `queued` or
`waiting` items. `--item <uuid>` may repeat, up to 100 unique ids per call.
`--all` selects one bounded batch of adoptable items from the blocking
population instead of naming ids; it is mutually exclusive with `--item` and
never selects a `running`, terminal, linked or retired-parent item. A contract:

- One transaction per batch, under the existing admission lock and item locks
  taken in a deterministic order.
- The domain status of the item is preserved: a `waiting` item stays waiting, so
  a new segment alone never bypasses the admission of its parent, and a Docling
  wait still has to be reconciled before it becomes eligible. The retry
  deadline, attempt count, dependency key, stage, resource and Docling remote
  reference are untouched.
- Every named parent must be active and not retired. `--all` skips a selected
  candidate that stopped being adoptable instead of refusing the batch; a named
  `--item` still refuses the whole batch when it is not eligible.
- Re-running an adoption on an item that already carries a segment is a no-op:
  no second segment, no new generation.
- A `running`, terminal, retired-parent or otherwise ineligible named item
  refuses the whole batch atomically, and a link that cannot be taken rolls the
  segment back rather than leaving one that no item points at.
- `--all` selects eligible rows in SQL before applying the batch bound, so any
  number of older ineligible rows cannot starve adoptable work. It reports
  `remaining_adoptable` (unlinked `queued`/`waiting` items with an active
  parent) and `total_blockers` (every unlinked non-terminal item). Repeat it
  only while `remaining_adoptable` is non-zero. `total_blockers` also includes
  running items, which require the explicit `finalize-interrupted --item`, and
  rows of a retired parent, which neither command can resolve; report those
  without claiming `adopt --all` can clear them. The command never loops to
  exhaustion by itself.

```
task_execution_cutover adopt --expect-database context69 --workers-stopped \
  --item 0f2c... --item 5b71...
```

```
task_execution_cutover adopt --expect-database context69 --workers-stopped --all
```

### `finalize-interrupted`

Closes explicitly named, unlinked `running` items whose domain lease is already
gone. Those items belong to a worker that stopped before the cutover, and the
tool removes them from the blocking population without replaying them:

- The item fails with the lease stage and the error
  `interrupted at task execution cutover`, and its open domain attempt becomes
  `interrupted` with a finished timestamp.
- The parent counts and status are recomputed in the same transaction, and the
  file status follows the existing failure projection.
- Nothing is requeued or replayed and no new segment is created. A later retry
  is an explicit user action that opens its own segment under the ordinary
  policy, and the retry flag follows that policy: an item with attempt budget
  left stays retryable, an exhausted one does not.
- A linked, non-running or still-leased item refuses the whole batch.

```
task_execution_cutover finalize-interrupted --expect-database context69 \
  --workers-stopped --item 0f2c...
```

## Procedure

1. Stop every task worker.
2. `inventory` and decide, per item, between `adopt` and
   `finalize-interrupted`.
3. Run the chosen command with the exact item ids, in batches of at most 100,
   or run `adopt --all` to hand over one bounded adoptable batch.
4. Repeat `inventory` (or `adopt --all` while its `remaining_adoptable` is
   non-zero) until no adoptable rows are left; route `total_blockers` that
   adoption cannot resolve through `finalize-interrupted` or report them.
5. Start the process again; startup prepares the library schema, the startup
   latch now reports ready and the task engine takes over.
