# Library file dedup lookup index

[简体中文](library-files-sha256-index.zh-CN.md)

The library dedup lookup reads `context69.library_files` by `(group_id,
sha256)` with a deterministic `ORDER BY created_at, id LIMIT 1`. Until now no
index covered that pair, so every miss that found no file scanned the whole
table. The migration `20261022140130_library_files_group_sha256_index.sql`
adds one non-unique btree index over `(group_id, sha256, created_at, id)`: the
equality pair leads the index and the trailing pair already carries the order
the lookup asks for, so the same index serves the filter and the sort.

The migration changes no query text, no response, and no uniqueness rule, and
it touches no data. It only makes the existing lookup an index lookup instead
of a full-table scan.

## What the migration does

The migration is one ordinary transactional file that runs inside the normal
`Database::connect` migration step:

1. A catalog guard looks for a same-name object
   `context69.idx_library_files_group_sha256`.
2. `CREATE INDEX IF NOT EXISTS idx_library_files_group_sha256 ON
   context69.library_files (group_id, sha256, created_at, id)` creates the
   index.

The guard is fail-closed, because `CREATE INDEX IF NOT EXISTS` only emits a
notice when a same-name relation already exists. Without the guard a wrong or
invalid object would make the statement a silent no-op and the upgrade would
report a success that never created the index.

The guard accepts a same-name object only when it is a valid, ready, live,
non-unique btree index over exactly `(group_id, sha256, created_at, id)` in the
default ascending order, with no predicate, no expression columns, and no
INCLUDE columns. Anything else aborts the migration with a message that names
the object and the properties that did not match.

The migration never drops, renames, or repairs a same-name object. Resolving
one is an operator decision.

## Pre-create the index before the upgrade

The statement inside the migration is a plain `CREATE INDEX`, so it takes a
write lock on `context69.library_files` for the whole build. On a large
library that blocks file ingestion and file writes until the build finishes.
This is the same trade-off the earlier `hot_path_indexes` migration made, and
the same way out: create the index concurrently, out of band, before the
upgrade.

Run this as the database owner, before the new version starts:

```sql
CREATE INDEX CONCURRENTLY idx_library_files_group_sha256
    ON context69.library_files (group_id, sha256, created_at, id);
```

`CREATE INDEX CONCURRENTLY` cannot run inside the migration's transaction, so
it has to be a separate session. The index name is never schema-qualified:
PostgreSQL always creates an index in the schema of its table, so this lands
in `context69` next to `context69.library_files`.

When the migration later runs, the guard finds the equivalent index, accepts
it, and the `IF NOT EXISTS` turns the statement into a no-op.

`CREATE INDEX CONCURRENTLY` does not hold a write lock for the build, but it
does wait for every transaction that is already running. A failed concurrent
build leaves an invalid index behind; see the recovery section.

## Verify the index

Run this before the upgrade and again afterwards:

```sql
SELECT c.relname,
       am.amname AS access_method,
       i.indisunique,
       i.indisvalid,
       i.indisready,
       i.indislive,
       i.indnkeyatts,
       i.indnatts,
       pg_get_indexdef(c.oid) AS definition
FROM pg_catalog.pg_index i
JOIN pg_catalog.pg_class c ON c.oid = i.indexrelid
JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
JOIN pg_catalog.pg_am am ON am.oid = c.relam
WHERE n.nspname = 'context69'
  AND c.relname = 'idx_library_files_group_sha256';
```

The expected result is one row with `access_method` `btree`, `indisunique`
false, `indisvalid`, `indisready` and `indislive` true, `indnkeyatts` 4,
`indnatts` 4, and a definition of

```sql
CREATE INDEX idx_library_files_group_sha256 ON context69.library_files USING btree (group_id, sha256, created_at, id)
```

`indnatts` larger than `indnkeyatts` means the index carries INCLUDE columns,
which the guard rejects. `indisvalid` false means the index is invalid, for
example after a failed concurrent build, and the guard rejects it too.

## When the guard refuses the upgrade

The migration aborts with a message of this shape:

```text
context69 library file dedup index guard failed for context69.idx_library_files_group_sha256: <the properties that did not match>. Inspect it with SELECT pg_get_indexdef('context69.idx_library_files_group_sha256'::regclass) and the pg_index/pg_class catalogs. ...
```

The whole migration rolls back, the ledger is not advanced, and the running
version stays on the previous one. Nothing was changed in the database, so the
upgrade can simply be retried once the object is resolved.

Resolve it in two steps:

1. Inspect the object with the verification query above and with
   `pg_get_indexdef`. The message lists the properties that did not match.
2. Decide, as the operator, which of the two paths applies:
   - the object is an equivalent index that the guard did not recognise — keep
     it only if the verification query shows the expected definition and
     validity, otherwise correct the definition yourself;
   - the object is wrong, invalid, or not an index at all — remove it yourself
     and re-create it with the `CREATE INDEX CONCURRENTLY` statement above, or
     let the migration create it by leaving no same-name object in place.

The migration itself never drops or repairs a same-name object, and it never
guesses which shape was intended. Removing an object under a name that another
component may use is an operator decision, not a migration behaviour.

## Applied migrations are immutable

Once a migration has been applied, its file must not be edited: the ledger
stores a checksum of the file and a changed file fails the next upgrade.
Fixes to a shipped migration are made in a new migration, and this index
needs none. The index is created by exactly one migration file, so a second
one would fight the first over the same name.
