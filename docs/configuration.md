# Configuration

## Config File Location

`context69` reads its main config file from the platform config directory:

- Linux: `~/.config/context69/config.toml`
- macOS: `~/Library/Application Support/context69/config.toml`
- Windows: `%APPDATA%\context69\config.toml`

## Main Sections

- `app_db`: Context69 metadata database
- `qdrant`: runtime vector store defaults or bootstrap values
- `embedding`: runtime embedding defaults or bootstrap values
- `docling`: optional bootstrap values for file parsing
- `scheduler`: sync scheduling defaults
- `mcp`: MCP server configuration
- `api`: HTTP API server configuration
- `app`: deployment-supplied application master secret
- `secret_store`: ciphertext key versioning for the encrypted secret store
- `connections[]`: bootstrap source connections imported into the app database on first startup
- `sources[]`: bootstrap source definitions imported into the app database on first startup

At runtime, the database is the source of truth for:

- runtime settings
- docling settings
- source connections
- source definitions

That means the backend can start with only `app_db.url`. When runtime settings are still
empty or invalid, Context69 boots in degraded mode so the frontend settings page can be used
to finish configuration. Search and library ingest require a restart after those settings are saved.

## Scheduler Options

- `max_concurrency`: shared task worker pool size and scheduler fan-out for translation, extraction, and sync. URL imports run through this shared pool. Default `2`.
- `valkey_url`: optional; enables persistent scheduler state and distributed execution leasing
- `execution_guard_ttl_secs`: lease TTL, default `30`
- `execution_guard_renew_interval_secs`: lease renewal interval, default `10`

The renew interval must be lower than the TTL. Runtime settings changes, including `max_concurrency`, take effect after the service restarts.

## Qdrant Options

- `recreate_on_dimension_mismatch`: when enabled, Context69 recreates the collection if the configured embedding dimension no longer matches the existing collection schema

## Docling Options

When `docling` is configured, Context69 uses:

- `cloudiful-docling-convert` for PDF and DOCX ingest
- Docling async JSON conversion for XLSX ingest

The supported config shape is:

- `docling.connection.base_url`
- `docling.connection.timeout_secs`
- `docling.connection.poll_interval_secs`
- `docling.connection.task_timeout_secs` (default `600`), the maximum time to wait for an async XLSX task after submission
- `docling.vlm.openai_base_url`
- `docling.vlm.api_key`
- `docling.vlm.vlm_pipeline_model`
- `docling.vlm.picture_description_model`
- `docling.vlm.code_formula_model`

The VLM block is optional. Leave all `docling.vlm.*` fields unset to disable VLM enrichment.
If you use raw Docling VLM config, set all five raw VLM fields together:

- `docling.vlm.openai_base_url`
- `docling.vlm.api_key`
- `docling.vlm.vlm_pipeline_model`
- `docling.vlm.picture_description_model`
- `docling.vlm.code_formula_model`

In the frontend settings page, configure Docling VLM directly with `openai_base_url`, `api_key`,
and the three model fields.

XLSX status and result requests retry transient connection errors, HTTP `429`, and `5xx`
responses with backoff. The async submission is not replayed, because the service may have
accepted it even when the response is lost.

Legacy OCR, PDF backend, image export, and enrichment toggle fields are no longer used.

## Secrets and Environment Overrides

Do not store production secrets in config files committed to source control.

The application database URL resolves as: explicit `--database-url` where
supported, then `DATABASE_URL`, then configured `app_db.url` / built-in default.

Useful environment overrides:

- `DATABASE_URL`
- `CONTEXT69_SCHEDULER__VALKEY_URL`
- `CONTEXT69_FILE_LIBRARY__TRUSTED_PROXY_ENABLED`

Any other nested config field can still be overridden with `__` separators.

Example:

```bash
export DATABASE_URL='postgres://user:pass@db/context69'
cargo run
```

If you prefer bootstrap-by-config instead of using the frontend, runtime-related overrides
such as `CONTEXT69_QDRANT__URL`, `CONTEXT69_EMBEDDING__API_KEY`, or Docling fields still work
and will be imported into the database on first startup.

## Application Master Secret

`app.master_secret` is the application-scoped deployment input that seals the
persisted reversible runtime secrets in the application database:

- `master_secret`: optional base64-encoded 32-byte key. Padding is optional.

It is one application-wide key that every encrypted application feature can use,
not a secret-store setting, so it is configured once under `[app]`. Supply it as
`CONTEXT69_APP__MASTER_SECRET` rather than in a config file committed to source
control. It is read from configuration, handed to the cipher, and never written
to PostgreSQL, a log line, an error, or a response.

`secret_store.key_version` stays where it is, because it describes the
ciphertext the store writes rather than naming a secret:

- `key_version`: the version new ciphertext is written under, default `1`. Must
  be greater than `0`.

Without `master_secret` the service still starts and still reads secrets that are
stored in the legacy plaintext representation, and any secret it creates is
stored the same way. A configured but unusable key is a startup configuration
failure and never degrades to that state. When the secret is present, a stored
sealed secret is either opened or reported as unconfigured; there is no
plaintext fallback for it.

### Master-key recovery

The master secret is not in the database. A PostgreSQL dump of the application
database therefore cannot recover a sealed value by itself: without the key every
sealed row is indistinguishable ciphertext, and there is no second copy anywhere
to fall back on. The dump and the key are two halves of one recovery plan and
have to be held under separate custody, neither of them in the same place as the
other.

A backup also does not cover the rest of the deployment. Qdrant collections and
object storage hold no sealed secret but do hold the indexed content, and neither
is inside a PostgreSQL backup. Snapshot them separately if they matter for
recovery.

### Master-key rotation

Rotation re-encrypts in place. `context69 rewrap-secrets` opens every sealed row
with the outgoing key and seals it again with the incoming one, at a strictly
higher key version. It deletes nothing, it stops at the first row it cannot
re-seal, and it enumerates its worklist by the *outgoing* key version — so a run
that stops can simply be run again: rows that already moved are skipped, and rows
that have not been reached still open with the outgoing key. That is what lets the
deployment keep serving until the cutover.

The mode runs before the application starts, so it opens no Valkey, no Qdrant,
no API, and no scheduler: it touches the application database and nothing else. It
logs bounded counts and never a key name, a value, a ciphertext, or a DSN.

The outgoing deployment is read from the ordinary configuration, exactly as a
serving process reads it, so the run always opens rows with the key that actually
sealed them. Only the incoming key is taken from the environment, and it is never
written to a config file, a command line, or the database:

| Variable | Role |
| --- | --- |
| `CONTEXT69_APP__MASTER_SECRET` | outgoing key |
| `CONTEXT69_SECRET_STORE__KEY_VERSION` | outgoing key version |
| `CONTEXT69_APP__NEXT_MASTER_SECRET` | incoming key, rewrap only |
| `CONTEXT69_SECRET_STORE__NEXT_KEY_VERSION` | incoming key version, rewrap only, strictly greater than the outgoing version |

### Rotation and recovery runbook

Run this from the same release as the deployment, so the schema matches. Replace
every placeholder; nothing below is a real credential or a real value.

1. **Record the outgoing key and version** and keep them retrievable. Until step 8
   the deployment still reads with the outgoing key, so a rollback has to stay
   possible.

2. **Take a custom-format backup and verify it**, from a host that can reach the
   database:

   ```bash
   pg_dump --format=custom --file=/secure/backup/context69-<timestamp>.dump "$DATABASE_URL"
   pg_restore --list /secure/backup/context69-<timestamp>.dump >/dev/null
   ```

   A plain-SQL or directory-format dump is not a substitute: step 3 needs
   `pg_restore`.

3. **Rehearse the restore into a disposable database.** Never restore over the
   live database to check a backup.

   ```bash
   createdb --template=template0 context69_restore_check
   pg_restore --dbname="$RESTORE_CHECK_DATABASE_URL" --exit-on-error \
     /secure/backup/context69-<timestamp>.dump
   ```

4. **Inspect the restored copy read-only.** Connect as a role with no write
   privilege, or wrap the statements in `BEGIN READ ONLY;` … `COMMIT;`. Select
   metadata columns only — never `value`, and never a key name:

   ```sql
   SELECT ciphertext_version, count(*) AS rows
   FROM context69.internal_secrets
   GROUP BY ciphertext_version
   ORDER BY ciphertext_version;

   SELECT key_version, count(*) AS rows
   FROM context69.internal_secrets
   WHERE ciphertext_version = 1
   GROUP BY key_version
   ORDER BY key_version;
   ```

   Both counts must match the source database, and every sealed row must be at
   the outgoing key version. A mismatch means the backup is not usable; stop here.

5. **Generate the incoming key and pick its version.** The version must be strictly
   greater than the outgoing one; a rewrap refuses to run otherwise. Generate the
   key with a cryptographic generator, for example:

   ```bash
   umask 077 && openssl rand -base64 32
   export CONTEXT69_APP__NEXT_MASTER_SECRET='<INCOMING_BASE64_KEY>'
   export CONTEXT69_SECRET_STORE__NEXT_KEY_VERSION='<INCOMING_VERSION>'
   ```

6. **Rehearse the rewrap on the disposable copy first.** Point `DATABASE_URL` at
   the restored database and run the same command; the counts in the log must add
   up to the sealed-row count from step 4.

   ```bash
   DATABASE_URL="$RESTORE_CHECK_DATABASE_URL" context69 rewrap-secrets
   ```

7. **Run the rewrap against the live database**, from a shell that already has the
   outgoing key in its normal configuration and the incoming key in the
   environment:

   ```bash
   context69 rewrap-secrets
   ```

   The log line reports `rewrapped` rows and how many categories contributed, and
   nothing else. A non-zero exit means a row could not be opened or re-sealed:
   fix the cause and run it again, which resumes.

8. **Verify, then switch over.** Verification is metadata-only and a read-only
   role is enough:

   ```sql
   SELECT key_version, count(*) AS rows
   FROM context69.internal_secrets
   WHERE ciphertext_version = 1
   GROUP BY key_version
   ORDER BY key_version;
   ```

   Every sealed row must now be at the incoming version and none may remain at the
   outgoing one. Then confirm the round trips on the service itself: deploy the
   same release with `CONTEXT69_APP__MASTER_SECRET` set to the incoming key
   and `CONTEXT69_SECRET_STORE__KEY_VERSION` set to the incoming version, and check
   that the settings projections still report `has_api_key`, `has_secret_key`, and
   `has_database_url`, that a search needing the rerank key still works, and that a
   document job needing the Docling VLM key still runs. Those read through the
   store, so a wrong key or a version left behind shows up as a failure rather
   than as a silently empty credential.

9. **Retire the outgoing key last.** Keep it retrievable until the recovery
   rehearsal, the backup verification, and a restore of the post-rewrap database
   have all succeeded. Only then remove it from the deployment and destroy the
   copy you kept.

### Reversible-secret backfill

`context69 backfill-secrets` seals the reversible credentials a deployment still holds
outside the store: the legacy `api_key` columns of the embedding, search, Docling and
shared translation/extraction settings, the source-connection database URLs, the
runtime S3 secret key, and any store row still in the legacy plaintext representation.
It is a manual maintenance mode with no automatic startup path.

| Flag | Meaning |
| --- | --- |
| *(none)* | read-only inventory: reports what a run would do and writes nothing |
| `--apply` | commit the migration; still the only write path |
| `--limit <count>` | how many changes one run may make; defaults to 500 and must be greater than `0` |

Both modes need `app.master_secret`: without it a run is refused before the first
row is read, because an unencrypted store cannot open the rows it would inventory.

The mode runs before the application starts, so it opens no Valkey, no Qdrant, no
API, and no MCP server, and it resolves no runtime credential. It reads each legacy
value raw, writes it sealed under its purpose and key version, and reads it back
before it touches anything legacy: then it nulls the four standalone nullable
credential columns and points each source connection at its sealed value. The
source-connection `database_url` and the runtime S3 `secret_key` keep their legacy
values — the first is `NOT NULL` with a non-blank check, the second belongs to a
settings projection whose readers need every field present.

What one run guarantees:

- **Seal before clear.** A legacy value is only cleared or referenced after its
  encrypted write committed and read back identically.
- **Read-only by default, and blind to nothing.** A dry run reads the store too, so
  it can tell an unmigrated column from a sealed one, and it stops on any row the
  configured key cannot open — exactly as a serving process would.
- **Stop on the first failure.** A run that cannot migrate one item does not skip
  it, and everything it already finished stays finished. The failing item keeps its
  legacy value, so re-running resumes.
- **Idempotent.** An item already sealed and cleaned up is neither rewritten nor
  counted again, so repeated runs converge and report nothing left.
- **Counts only.** The log line reports `scanned`, `absent`, `already_sealed`, `sealed`,
  `cleared`, `referenced`, `retained`, `unmapped`, and `limit_reached` — never a value,
  key name, DSN, or connection name.

Run it from the same release as the deployment, with the service stopped, so
nothing rewrites a legacy column while it runs:

1. **Take and verify a backup** as in the rotation runbook above, and rehearse the
   restore into a disposable database.
2. **Rehearse on the disposable copy**, with the deployment's own
   `app.master_secret` still configured, and run the mode with no flags: the
   counts are the work the apply run would perform. The rehearsal must use the key the
   deployment seals under; another valid key also writes ciphertext, but opens none of
   the copy's rows, so a green run under it proves nothing about reading them back.
3. **Apply on the live database.**

   ```bash
   context69 backfill-secrets --apply
   context69 backfill-secrets --apply --limit 100
   ```

   Repeat while a run reports `limit_reached=true`. A run that stops with an error
   exits non-zero and names the purpose that could not be migrated: fix the cause
   and run it again.
4. **Verify, metadata-only**, with a read-only role:

   ```sql
   SELECT ciphertext_version, count(*) AS rows FROM context69.internal_secrets
   GROUP BY ciphertext_version ORDER BY ciphertext_version;

   SELECT count(*) AS unreferenced_source_connections
   FROM context69.runtime_source_connections WHERE database_url_secret_key IS NULL;
   ```

   Every credential this phase owns must be sealed, and no source connection may be
   left without a reference.

### Restoring a database after a lost master secret

If the outgoing key is gone and no rewrap has run, the sealed rows in a restored
database are unrecoverable: the dump holds ciphertext and the key is not in it.
Recovery is then a re-provisioning exercise — re-enter each credential through the
settings API, which writes it sealed under the current key — not a restore. This is
why step 1 keeps the outgoing key, and why the backup and the key are stored
separately.


## Trusted URL Import Proxy

URL imports ignore proxy environment variables by default. Enable
`file_library.trusted_proxy_enabled` from Runtime Settings, or use
`CONTEXT69_FILE_LIBRARY__TRUSTED_PROXY_ENABLED=true` during initial bootstrap, to trust the
deployment's HTTPS egress proxy. The setting takes effect for new downloads immediately.

When enabled, Context69 uses `HTTPS_PROXY`/`https_proxy`, falling back to
`ALL_PROXY`/`all_proxy`, and applies `NO_PROXY`/`no_proxy`. The proxy endpoint is resolved and
validated separately from the requested public URL. The egress proxy must enforce destination
network isolation because it performs the final target connection.

## URL Import Queue

URL imports are processed through the shared task worker pool controlled by
`scheduler.max_concurrency` (see Scheduler Options). `file_library.url_import_min_interval_ms`
controls the minimum interval between requests to the same `scheme://host:port` (default
1000 ms). `file_library.ingest_concurrency` and `file_library.url_import_concurrency` remain
stored, validated, and exposed for backward compatibility but no longer control task worker
count; URL imports use the shared pool with per-host throttling via the rate limiter. Without
`scheduler.valkey_url`, throttling is local to each process; multi-instance deployments must
configure the same Valkey URL on all instances. A Valkey limiter initialization failure does
not fall back to local throttling, so queued URL jobs remain queued until the shared limiter
is available. Runtime settings changes take effect after the service restarts.

## SQLx CLI

SQLx macros and `cargo sqlx prepare` read `DATABASE_URL`.
`cargo run --bin db_init -- --database-url ...` has highest priority.
Without that flag, `db_init` loads root `.env` if present, then resolves
`DATABASE_URL`, and finally `app_db.url`.

For local development, set `DATABASE_URL` as the canonical value:

```bash
export DATABASE_URL='postgres://postgres:postgres@127.0.0.1:5432/context69'
cargo run --bin db_init
cargo sqlx prepare --workspace -- --all-targets
```
