# Task execution cutover

Task items are executed by the `cloudiful_tasks` library in the `context69`
namespace and `items` queue. Each item segment carries the immutable admission
class and resources derived from its parent task kind:

- `text_batch` → `interactive`
- `file_batch`, `url_batch` → `bulk`
- every other current kind → `maintenance`

Every segment also declares the shared `ingest` resource. The global queue
concurrency remains authoritative; per-class and shared-ingest policies can only
narrow it.

The `task_execution_cutover` tool is read-only only for `inventory`. Since it
connects without migrations or upgrade admission, it performs no DDL. It reads
`DATABASE_URL` from the environment only, and both the configured and the
server-reported database name must equal the mandatory `--expect-database`
value. Every write command requires `--workers-stopped`, refuses a live lease,
and refuses a running target.

## Inventory

`inventory` is the read-only form. It reports one bounded page of unlinked
non-terminal items with their parent, status, kind, ordinal, retirement,
generation, link and Docling-wait diagnostics:

```sh
task_execution_cutover inventory \
  --limit 100 \
  --offset 0 \
  --expect-database context69
```

`--limit` is 1..100 and `--offset` is non-negative. `inventory` takes no
`--item`, `--all` or `--workers-stopped`, and its operator connection refuses
writes.

## Adopt abandoned items

`adopt` links unlinked queued or waiting items of an active, non-retired parent
to a fresh library segment. Stop every task worker first and name items
explicitly, or use the bounded `--all` selection:

```sh
task_execution_cutover adopt \
  --item 00000000-0000-0000-0000-000000000000 \
  --workers-stopped \
  --expect-database context69

task_execution_cutover adopt \
  --all \
  --workers-stopped \
  --expect-database context69
```

At most 100 unique items may be named. Each batch runs in one transaction under
the admission lock and sorted item locks; a parent that is deleted or no longer
queued/running/waiting is refused, and the whole batch rolls back. A bounded
`--all` also reports `remaining_adoptable` and `total_blockers`. Re-running an
adoption is idempotent: already linked items are counted and left alone.

## Finalize interrupted items

`finalize-interrupted` closes explicit unlinked running items whose library
mirror lease is gone. It fails the item with the cutover error, marks the open
domain attempt interrupted, clears the mirror, and never requeues or replays the
item. Stop every task worker first and name the items:

```sh
task_execution_cutover finalize-interrupted \
  --item 00000000-0000-0000-0000-000000000000 \
  --workers-stopped \
  --expect-database context69
```

The batch is all or nothing. A later retry is an explicit user action that
opens its own segment under the existing retry policy.

## Reclassify unclassified backlog

Operators can convert the bounded backlog of queued or waiting items whose
current segment still has no class. Stop every task worker first, then run the
tool against the expected database:

```sh
task_execution_cutover reclassify-admission \
  --limit 100 \
  --workers-stopped \
  --expect-database context69
```

The command processes at most the requested number of eligible items (1..1000).
Each item is fenced by its current pointer and generation. In one transaction
the old queued segment is cancelled, a fresh next-generation segment is produced
with the class and resources of the parent kind, and the item is relinked only
if the old pointer, generation and terminal state still match. A changed item
rolls the whole batch back.

The command skips items with an unresolved Docling `submitting` or
`submission_uncertain` row without cancelling or probing them. It reports the
processed, relinked, skipped and remaining counts. Repeat only while remaining
is non-zero; a completed run is a no-op.

The command never runs automatically at startup, never changes terminal history,
and never touches running work. A running target or a live lease is refused.
This is an explicit, bounded, fail-closed operator action.

## Verification

Inspect the target read-only before running any database-backed verification,
then authorize the reset flow explicitly. A bare `--target` selection is not an
executable database command; it narrows a run that still needs the guarded
invocation:

```sh
./scripts/test-backend.sh --preflight
./scripts/test-backend.sh --authorize-reset
```

The cutover tool's exit status is `0` for success, `2` for a usage or target
mismatch, and `3` for a database or safety refusal. It never runs migrations or
touches an unexpected database.
