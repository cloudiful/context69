//! Pure contract test for the guarded library-file dedup index migration
//! (issue #1041 P1).
//!
//! The test needs no database and no extra dependency: it inspects the
//! migration this crate actually compiles, so a guard that stops matching the
//! shipped SQL, a migration that stops being transactional, a reused
//! timestamp, or a second migration creating the same index name fails here
//! instead of during a production upgrade.

use context69_db_schema::MIGRATOR;

const MIGRATION_DESCRIPTION: &str = "library files group sha256 index";
const INDEX_NAME: &str = "idx_library_files_group_sha256";
const INDEX_DEFINITION: &str = "ON context69.library_files (group_id, sha256, created_at, id)";
const EXPECTED_COLUMNS: &str = "ARRAY['group_id', 'sha256', 'created_at', 'id']";

fn migrator() -> &'static sqlx::migrate::Migrator {
    &MIGRATOR
}

/// The one compiled migration that owns the dedup lookup index.
fn dedup_index_migration() -> &'static sqlx::migrate::Migration {
    let matches: Vec<&'static sqlx::migrate::Migration> = migrator()
        .iter()
        .filter(|migration| migration.description == MIGRATION_DESCRIPTION)
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "the compiled migrator must embed exactly one `{MIGRATION_DESCRIPTION}` migration"
    );
    matches[0]
}

/// The migration body without its `--` comment lines.
fn executable_sql(sql: &str) -> String {
    sql.lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<&str>>()
        .join("\n")
}

/// The statement that starts at `marker`, up to its terminating semicolon.
fn statement_from(sql: &str, marker: &str) -> String {
    let start = sql
        .find(marker)
        .unwrap_or_else(|| panic!("the migration must contain `{marker}`"));
    sql[start..]
        .split(';')
        .next()
        .unwrap_or_default()
        .to_owned()
}

#[test]
fn the_compiled_migrator_embeds_one_transactional_dedup_index_migration() {
    let migration = dedup_index_migration();
    assert!(
        !migration.no_tx,
        "the dedup index migration must stay transactional"
    );
    assert!(
        !migration.sql.as_str().starts_with("-- no-transaction"),
        "the dedup index migration must not opt out of the transaction"
    );
}

#[test]
fn the_dedup_index_migration_version_is_unique_and_runs_after_the_library_table() {
    let migration = dedup_index_migration();
    let versions: Vec<i64> = migrator().iter().map(|entry| entry.version).collect();
    assert_eq!(
        versions
            .iter()
            .filter(|version| **version == migration.version)
            .count(),
        1,
        "a reused migration timestamp would make the ledger ambiguous"
    );
    let library_table_version = migrator()
        .iter()
        .find(|entry| {
            executable_sql(entry.sql.as_str())
                .contains("CREATE TABLE IF NOT EXISTS context69.library_files")
        })
        .map(|entry| entry.version)
        .expect("the library_files table migration is compiled");
    assert!(
        migration.version > library_table_version,
        "the dedup index migration must be applied after the library_files table exists"
    );
}

#[test]
fn the_dedup_index_migration_creates_the_exact_nonunique_lookup_index() {
    let executable = executable_sql(dedup_index_migration().sql.as_str());
    let create = statement_from(&executable, "CREATE INDEX");
    assert!(
        create.contains(&format!("CREATE INDEX IF NOT EXISTS {INDEX_NAME}")),
        "the migration must create the index by its exact name: {create}"
    );
    assert!(
        create.contains(INDEX_DEFINITION),
        "the index must cover the dedup equality pair and its lookup order: {create}"
    );
    assert!(
        !create.contains("UNIQUE"),
        "the dedup lookup index must not add a uniqueness rule: {create}"
    );
    assert!(
        !executable.contains("CONCURRENTLY"),
        "CREATE INDEX CONCURRENTLY cannot run inside the migration transaction"
    );
}

#[test]
fn the_dedup_index_migration_guards_the_same_name_object_before_creating_it() {
    let executable = executable_sql(dedup_index_migration().sql.as_str());
    let guard_start = executable
        .find("DO $$")
        .expect("the same-name compatibility guard is present");
    let create_start = executable
        .find("CREATE INDEX")
        .expect("the index statement is present");
    assert!(
        guard_start < create_start,
        "the compatibility guard must run before the index is created"
    );
    let guard = &executable[guard_start..create_start];
    for fragment in [
        "pg_catalog.pg_class",
        "pg_catalog.pg_namespace",
        "pg_catalog.pg_index",
        "'context69'",
        INDEX_NAME,
        "indisvalid",
        "indisready",
        "indislive",
        "indisunique",
        "indpred",
        "indexprs",
        "indnkeyatts",
        "indnatts",
        EXPECTED_COLUMNS,
        "RAISE EXCEPTION",
    ] {
        assert!(
            guard.contains(fragment),
            "the compatibility guard must check `{fragment}`"
        );
    }
    assert!(
        !guard.contains("DROP "),
        "the migration must never drop or repair a same-name object"
    );
    assert!(
        !guard.contains("CREATE INDEX"),
        "the guard must inspect the catalog instead of creating the index"
    );
}

#[test]
fn no_other_compiled_migration_creates_the_dedup_index() {
    let creators = migrator()
        .iter()
        .filter(|entry| {
            let executable = executable_sql(entry.sql.as_str());
            executable.contains(&format!("CREATE INDEX IF NOT EXISTS {INDEX_NAME}"))
                || executable.contains(&format!("CREATE INDEX {INDEX_NAME}"))
        })
        .count();
    assert_eq!(
        creators, 1,
        "exactly one compiled migration may create `{INDEX_NAME}`"
    );
}
