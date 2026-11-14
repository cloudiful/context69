# Persistence architecture

How Context69 stores and reads data today: the native SeaORM paths, the SQLx
statements that are still retained, and what owns each retained statement.

## One pool, two clients

- SQLx (`sqlx::PgPool`) serves every statement that is not native.
- SeaORM 2.0.4 (`sea_orm::DatabaseConnection`) serves the entity paths.

`src/db/mod.rs` builds one `PgPool` per connection mode (`connect`,
`connect_read_only`, `connect_operator`) and converts a clone of it into the
native connection with `pool.clone().into()`. Both clients therefore share the
pool limits, lifetime, credentials and transaction visibility; there is no
second pool and no second connection string.

Native query expressions are built with `sea_query` and executed through
SeaORM. A SeaQuery expression is native ORM usage even when the statement it
produces is plain SQL text.

## Native entity models

| Module | Models | Tables |
| --- | --- | --- |
| `src/db/entity/` | 29 | `docling_settings`, `document_chunks`, `documents`, `git_provider_connections`, `git_repository_sources`, `git_webhook_registrations`, `group_memberships`, `groups`, `library_dependency_gates`, `library_file_documents`, `library_files`, `library_folders`, `library_legacy_object_cleanup`, `library_storage_objects`, `personal_access_tokens`, `rerank_item_scores`, `runtime_chunking_settings`, `runtime_code_embedding_settings`, `runtime_embedding_settings`, `runtime_file_library_settings`, `runtime_qdrant_settings`, `runtime_scheduler_settings`, `runtime_source_connections`, `search_generations`, `search_settings`, `source_checkpoints`, `source_configs`, `sync_runs`, `users` |
| `crates/context69-translation/src/entity/` | 5 | `document_translation_attempts`, `document_translation_jobs`, `group_translation_settings`, `translation_provider_settings`, `translation_provider_usage` |
| `crates/context69-secret-store/src/entity/` | 1 | `internal_secrets` |

The native call sites are the modules that own those domains:
`src/db/documents.rs`, `src/db/git_repositories/{connections,repositories,webhooks}.rs`,
`src/db/{auth,namespaces,search_cache,search_settings,docling_settings,personal_access_tokens,runtime_settings,source_connections,sync_runs}.rs`,
`src/library_store/{files,folders,objects,documents,dependency_gates,source_release}.rs`,
`src/source_store/queries.rs`, `crates/context69-translation/src/store{,/jobs}.rs`
and `crates/context69-secret-store/src/database.rs`.

Array reads and array-guarded writes are native too, so an array parameter is
not by itself a reason to keep a statement in SQLx: `list_files_by_ids`
(`src/library_store/files.rs`), `list_storage_paths_for_files`
(`src/library_store/documents.rs`), `list_rerank_item_scores`
(`src/db/search_cache.rs`), `touch_personal_access_tokens_last_used`
(`src/db/personal_access_tokens.rs`) and `list_document_ids_by_keys`
(`src/db/documents.rs`) all bind arrays natively.

## Schema ownership

The legacy SQLx migration chain is the only schema executor.

- `migrations/` holds the applied migration files and their checksums. The
  `_sqlx_migrations` ledger records what has been applied and is the single
  authority for whether a schema has run.
- `src/db/mod.rs` runs the migrator through `sqlx::migrate::Migrator` at
  startup, after `src/db/upgrade_admission.rs` classifies the pending upgrade.
  That admission check reads the ledger and the migration guard predicates and
  never writes.
- `crates/context69-db-schema` bootstraps the separate `cloudiful_tasks`
  library schema at the same point, and `src/bin/db_init.rs` is the standalone
  initializer entry point.

Each entity model exposes a `schema()` builder that declares the columns,
types, defaults, primary key and owned indexes as they exist after the applied
migrations. Those declarations are asserted in unit tests and are never
executed. The identity clause, the check constraints and the optional
`pg_trgm` indexes stay owned by the migration files: the ORM never creates an
extension and never emits DDL.

Foreign-key ownership is unchanged. The models declare empty relation enums,
so referential integrity, cascade behaviour and every constraint remain owned
by the existing schema exactly as the migrations define them.

## Retained SQLx statements

| Family | Where | Why it stays SQLx |
| --- | --- | --- |
| Shared task, library and publication transactions | `src/db/tasks/**`, `src/library_store/**`, `src/sql/db/tasks/**`, `src/sql/library_store/**` | One transaction spans borrow counters, fence rows, publication and cancellation state. That transaction is owned by SQLx transaction code today. SeaORM can run transactions as well, so this is a present ownership boundary rather than a technical impossibility; moving it means re-deriving the same atomicity and borrow/release bookkeeping. |
| Recursive hierarchy queries and advisory locks | `src/sql/db/namespaces/**`, `src/sql/library_store/folders/descendant_*.sql`, `src/sql/db/git_repositories/lock_git_group_source_ancestors.sql`, and the advisory-lock statements such as `src/sql/db/tasks/lock_task_admission.sql`, `src/sql/db/tasks/handover_lock.sql`, `src/sql/library_store/objects/lock_storage_object.sql`, `src/sql/db/git_repositories/acquire_git_connection_creation_lock.sql`, `src/sql/db/namespaces/lock_group_move_ownership.sql`, `src/sql/db/library_metadata_mirror/session_lock.sql` | Recursive CTEs over the group hierarchy are self-referential, and `pg_advisory_lock` sessions are session-scoped: the lock lifetime has to match the surrounding transaction. |
| Ledger and upgrade admission | `migrations/**`, `src/db/upgrade_admission.rs`, `src/bin/db_init.rs`, `crates/context69-db-schema/`, the ledger gate in `src/services/library/artifact_backfill.rs` | The ledger is the SQLx migration ledger. Reading it, gating on it and applying it belongs to the migration runner. |
| Bulk fixed-array statements | `src/sql/db/documents/insert_document_chunks_bulk.sql`, `src/sql/db/documents/insert_rebuild_staging_chunks.sql`, `src/sql/db/documents/verify_document_chunk_batch.sql`, `src/sql/db/metadata_indexes/*_bulk.sql`, `src/sql/db/git_repository_files/{insert_git_generation_files,replace_git_generation_files,replace_git_file_chunks,seed_git_generation_files}.sql`, `src/sql/db/search_registry/delete_original_entries_batch.sql` | These statements bind the whole batch in one statement. The writes keep the all-or-nothing and duplicate handling of that single statement, and the batch check keeps the same one-statement view of the batch. |
| Signed limit/offset fallback in native methods | `src/db/auth.rs` (`list_users`, `search_user_directory`), `src/source_store/queries.rs` (`list_sources_page`), `src/db/git_repositories/repositories.rs` (`list_due_git_refresh_sources`, `list_pending_git_deletions`) | The ORM builder takes an unsigned limit and offset. A negative signed value has no native representation, so the method runs the retained SQLx statement for that case and the native path otherwise. |
| Source dynamic SQL | `src/sources/postgres_sql.rs` | User-defined SQL sources run operator-authored statements and stay raw SQL by design. |
| Legacy secret backfill | `crates/context69-secret-store/src/database/backfill.rs` with `crates/context69-secret-store/src/sql/internal_secrets/{claim_legacy_internal_secrets,count_legacy_internal_secrets,seal_legacy_internal_secret}.sql` | The backfill claims, counts and seals pre-existing plaintext secrets as a separate operator-invoked pass with its own statements. |
| Guarded last-administrator mutations | `src/db/auth.rs` with `src/sql/db/auth/{count_active_admin_users,update_user,set_user_disabled_at,get_user_for_update_by_login_name}.sql` | Clearing the admin flag or disabling a user must observe the active-administrator count in the same transaction and refuse to remove the last administrator, so the guarded count and the guarded update stay one statement pair. |

### Retained statements without a native model

These are ordinary table CRUD that has no entity model behind it. They are a
conversion gap, not a semantic requirement:

- `src/sql/db/document_results/**` — result insert, list and replay.
- `src/sql/db/deleted_entities/**` — id-tombstone rows and purge marking.
- `src/sql/db/vector_index_state/**` and
  `src/sql/db/vector_collection_retirement/**` — vector index bookkeeping.
- `src/sql/db/library_ingest_runs/**` — ingest run and section bookkeeping.
- `src/sql/db/metadata_payload_sync/**` and
  `src/sql/db/library_metadata_mirror/**` — payload sync and metadata mirror
  tables.
- `src/sql/db/translations/**` — the translation version, chunk and rebuild
  staging tables. The job, attempt, provider-settings and usage tables are
  native in `crates/context69-translation`.
- `src/sql/db/git_repositories/**` and `src/sql/db/git_repository_files/**` —
  the repository, generation and file rows beyond the native Git models
  (connections, repository sources, webhook registrations).
- `src/sql/db/auth/list_users.sql` and `src/sql/db/auth/search_user_directory.sql`
  — the user directory listing and search. `users` has a native model; these
  two statements remain because they are the fallback for a negative signed
  limit or offset.

Other retained statements target tables that do have a native model and belong
to the shared flows above: the document upsert, business-field update, chunk
verification, search and rebuild statements under `src/sql/db/documents/**`,
the rebuild and registration flows under `src/sql/db/search_registry/**`, the
metadata index values under `src/sql/db/metadata_indexes/**`, the group
hierarchy and access-scope statements under `src/sql/db/namespaces/**`, the
sync-run statements under `src/sql/db/sync_runs/**` and the library store
flows under `src/sql/library_store/**`.

The runtime settings tables have native models, native reads and native writes
for several of them (`src/db/runtime_settings.rs`, `src/db/docling_settings.rs`,
`src/db/search_settings.rs`, `src/db/source_connections.rs`). The combined save
of all runtime settings writes every settings table in one transaction through
`src/sql/db/runtime_settings/save_*.sql`, so that transaction stays in SQLx.

## Secret-store error mapping

`crates/context69-secret-store/src/database.rs` exposes a public
`Result<_, sqlx::Error>` API while the implementation is native. The mapper
takes a `sea_orm::DbErr` and returns the underlying `sqlx::Error` unchanged
when the driver error is still uniquely owned, using `Arc::try_unwrap` on the
error the ORM holds. When the error is shared, or when it is not a driver
error at all, the mapper falls back to `sqlx::Error::Protocol` with the ORM's
message. That fallback is a boundary of the mapping: on it the caller sees a
protocol error instead of the original variant, while the normal path keeps
the exact `sqlx` variant and SQLSTATE.

## Retained tooling

Retained statements still need the SQLx metadata tooling, so it stays:

- the `sqlx` dependency with the `query_file*!` macros,
- `.sqlx/` with the offline query metadata,
- `sqlx.toml` with the macro's preferred crates,
- `build.rs`, which declares the root `src/sql` query files and the metadata
  cache as compiler inputs so a query edit invalidates both Cargo and the
  content-addressed compile cache,
- `scripts/check-sqlx-metadata.py`, which checks the committed metadata against
  a fresh `cargo sqlx prepare --check`.

## Inventory

The counts below are an inventory of the tree, useful as a diff reference. They
say how many artifacts exist; they do not say how much of the persistence
surface is covered. Baseline is `ade6561`; the same counts hold for the
earlier baseline `122d254`, which differs only in frontend files.

```sh
# native entity model tables
rg --no-filename -oP 'table_name\s*=\s*"\K[a-z_]+' src/db/entity crates/*/src/entity | sort -u | wc -l

# retained statement files under src/ and crates/
rg --files -g '*.sql' src crates | wc -l
git ls-tree -r --name-only ade6561 src crates | rg -c '\.sql$'

# offline SQLx metadata entries
rg --files .sqlx | wc -l
git ls-tree -r --name-only ade6561 .sqlx | wc -l

# SQLx macro call sites
rg -o --no-filename '\b(query_as!|query!|query_scalar!|query_file_as!|query_file_scalar!|query_file!)' -g '*.rs' src crates | wc -l
git grep -o -E '\b(query_as!|query!|query_scalar!|query_file_as!|query_file_scalar!|query_file!)' ade6561 -- src crates | wc -l
```

| Artifact | Baseline | Current |
| --- | --- | --- |
| Native entity model tables | 0 | 35 |
| Retained `.sql` files under `src/`, `crates/` | 619 | 488 |
| Applied migration files | 131 | 131 |
| Offline SQLx metadata entries | 613 | 484 |
| SQLx macro call sites | 711 | 553 |

## Validation and its limits

The persistence layer is covered by the workspace gates:

```sh
SQLX_OFFLINE=true cargo build --workspace --offline
SQLX_OFFLINE=true cargo clippy --workspace --all-targets --offline
SQLX_OFFLINE=true cargo test --workspace --offline
cargo fmt --all -- --check
```

The suite is unit-only. It opens no database connection and covers query
builders, projections, guards and error mapping in isolation. There is no
live-database parity check, so the statements and the entity declarations are
verified against the applied schema by reading the migration files, not by
running them against PostgreSQL. A statement or declaration that only fails
against a live database is not covered by the automated gates.
