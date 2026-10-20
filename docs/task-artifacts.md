# Task artifact storage

[简体中文](task-artifacts.zh-CN.md)

Large task inputs and checkpoints live in the content-addressed library storage
instead of `task_items.payload`. This page describes the stored layout and the
operator tool that moves terminal data in either direction.

## What is stored

An artifact reference is a row in `context69.task_item_artifacts` with the item,
one kind, one encoding and one storage-object id:

| kind | encoding | payload field it replaces |
| --- | --- | --- |
| `download` | `raw-v1` | `download_artifact.content_base64` |
| `sections` | `ingest-sections-json-v1` | `section_payload` |

The registry `context69.library_storage_objects` stays authoritative for the
group, SHA-256, size, backend and object key. The artifact row stores no copy of
those values, and the group is always derived from the item's parent task.

The task pipeline reads inline data first and falls back to a verified artifact;
a reference that cannot be read back is an error, never "no data". Re-runs and
retries copy references instead of re-uploading, and file, task-input and
artifact references all protect the same object.

Storage releases run through the existing machinery: deleting or retargeting a
reference enqueues an old-object cleanup in `library_storage_object_cleanup`,
staged objects keep their 24-hour grace, and an object shared by a library file,
a task input or another item survives until the last reference is gone.

## Operator tool

The audited maintenance runner ships inside the application image as
`context69 task-artifacts`. It is dispatched before configuration loading,
database admission, migrations, workers and signal handlers, has no HTTP or UI
surface, and never migrates the database. The database URL is read from
`DATABASE_URL` or the repository `.env`; it is never accepted on the command
line. `task_artifact_backfill` is the same implementation as a standalone
development binary, built by `cargo build --bin task_artifact_backfill`.

Both directions preview by default and only write with `--apply`. Preview uses a
non-migrating operator pool whose PostgreSQL sessions are started with
`default_transaction_read_only=on`, and it never reads object content.

Development builds can run the standalone binary with the same flags:
`cargo run --bin task_artifact_backfill -- --help`.

### Forward: inline to storage

```sh
context69 task-artifacts                          # preview
context69 task-artifacts --apply --acknowledge-reader-upgrade
```

Forward apply requires both the artifact relation and the applied migration
`20261019104722` with the checksum compiled into the binary. It then stages one
field at a time through the content-addressed store, reads it back, verifies the
registry identity, and commits the reference and the inline removal together.
Only the target field is removed; every other payload key, including
`indexing_checkpoint`, is preserved.

Removing inline bytes means a build without artifact-aware readers cannot read
those items, so the acknowledgement states that the artifact-aware version must
stay deployed. A repeated run reports `already_externalized` and changes
nothing.

### Inverse: storage to inline

```sh
context69 task-artifacts --restore-inline          # restore preview
context69 task-artifacts --restore-inline --apply --acknowledge-inline-growth
```

Restore mode scans existing references, reads each object through the verified
reader (size, digest, backend and encoding from the registry), and restores only
a field that is currently missing. A `download` field is re-encoded as Base64
and requires the existing `download_artifact` metadata and a declared SHA-256
that equals the artifact digest; no source URL, filename or other identity is
invented. A `sections` field must parse as the existing ingest-section array and
keeps the rest of the payload untouched.

The restore statement locks the parent task before the item, rechecks the
terminal item and parent, the absent lease, the group, the exact reference
identity and the missing field, then writes the field and deletes exactly that
reference in one statement. A new claim, a changed reference, a group change or
an inline value that appeared in the meantime yields no mutation and a reported
skip. Restored bytes are never overwritten.

Restoring inline data grows PostgreSQL. The report counts restored encoded
bytes, and the byte caps and the total budget are checked before allocating the
Base64 text. It also prints a point-in-time count of remaining references with
an explicit `point_in_time` marker: an online application can create new
references after that query, so a zero is a snapshot, not a rollback
certificate, and restore mode never rolls back the schema or the migration
ledger.

### Limits, caps and skips

Both directions share the same bounds:

- `--kind download|sections|all` (default `all`);
- repeatable `--item-id` for a bounded selection;
- `--page-size` 1-1000 (default 100) and `--max-items` (default 500);
- `--max-artifact-bytes` capped at 256 MiB, and `--max-total-bytes`
  (default 512 MiB);
- `--delay-ms` between applied items (default 100);
- `--resume-cursor <uuid:kind>` to continue after the last scanned pair.

A capped page or item limit sets `scan_complete: false` and returns the cursor of
the last scanned pair, not of the last success. Skips are reported per class,
for example `active_item`, `no_group`, `group_mismatch`, `inactive_backend`,
`invalid_shape`, `oversize`, `missing_declared_sha`, `invalid_declared_sha`,
`sha_mismatch`, `inline_present`, `already_externalized`, `no_inline`,
`changed_ref` and `race_changed`. Unexpected storage, SQL or integrity errors
stop the batch with a redacted aggregate and a non-zero exit; they are never
reported as skips.

The report contains counts, byte totals, a bounded sample of item ids and kinds,
and explicitly `input_validation: not_performed` for a preview. It never prints
payload content, filenames, URLs, digest values or credentials.

## Safety notes

- Preview is metadata-only; it performs no object upload, download or database
  write.
- Apply needs the artifact-aware version deployed. Restoring inline data does
  not make an older application version safe: the old migrator can reject a
  ledger that contains a newer applied migration, and there is no down
  migration. Before considering an older version, quiesce all artifact writers
  and validate the deployment and schema-compatibility plan.
- Part of the database space freed by forward apply can be reused by PostgreSQL
  but does not shrink the data files by itself. Do not run `VACUUM FULL` or
  delete S3 objects or WAL by hand as part of this tool.
- Reading one JSONB field still detoasts that value in PostgreSQL for the row.
  The client keeps at most one capped field and one capped decoded buffer per
  item and kind.
- Local filesystem verification in tests proves the tool and the round trip; it
  does not substitute for a production S3 verification, and this tool never runs
  automatically against production.
