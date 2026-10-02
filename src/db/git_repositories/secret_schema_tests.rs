//! Schema invariants for the Git secret reference seam (issue #681 work unit
//! 4A3b-5).
//!
//! These cover the Git-purpose half of the secret store's persistence contract:
//! that the App private key arrives as one additive nullable reference which no
//! applied migration is retrofitted into, that a secret rotation moves exactly
//! one reference column and cannot reach a lifecycle column, and that a
//! reference update is confined to the group that owns the record.
//!
//! They are split from `schema_tests` because the ownership, generation, and
//! content invariants there are about a different subject and are already
//! sizeable. Every statement and migration is read with `include_str!` and
//! asserted from its own text, so nothing here needs a database, and the
//! helpers below are deliberately copies rather than shared items: a
//! `schema_tests` assertion must not be able to be weakened by an edit here, or
//! the other way round.

const GROUPS_MIGRATION_SQL: &str =
    include_str!("../../../migrations/20261001010156_git_repository_groups_generations.sql");
const PHASE2_MIGRATION_SQL: &str =
    include_str!("../../../migrations/20260930204952_git_repository_sources.sql");
const APP_KEY_MIGRATION_SQL: &str =
    include_str!("../../../migrations/20261002025238_git_connection_app_private_key.sql");
const UPSERT_CONNECTION_SQL: &str =
    include_str!("../../sql/db/git_repositories/upsert_git_provider_connection.sql");
const GET_CONNECTION_SQL: &str =
    include_str!("../../sql/db/git_repositories/get_git_provider_connection.sql");
const INSERT_CONNECTION_SQL: &str =
    include_str!("../../sql/db/git_repositories/insert_git_provider_connection.sql");
const LIST_CONNECTIONS_SQL: &str =
    include_str!("../../sql/db/git_repositories/list_git_provider_connections.sql");
const SET_CONNECTION_CREDENTIAL_SQL: &str =
    include_str!("../../sql/db/git_repositories/set_git_connection_credential_secret_key.sql");
const SET_CONNECTION_APP_KEY_SQL: &str =
    include_str!("../../sql/db/git_repositories/set_git_connection_app_private_key_secret_key.sql");
const SET_WEBHOOK_SIGNING_SECRET_SQL: &str =
    include_str!("../../sql/db/git_repositories/set_git_webhook_signing_secret_key.sql");

/// The executable text of `sql`, with `--` comment lines removed.
///
/// A test asserting what a statement *does* must not be satisfied or defeated by
/// prose about it, so every content check below reads the executable text.
fn executable(sql: &str) -> String {
    sql.lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The text between two markers, so a test can inspect one clause of a
/// statement rather than the whole file.
fn between<'a>(sql: &'a str, start: &str, end: &str) -> &'a str {
    let from = sql
        .find(start)
        .unwrap_or_else(|| panic!("statement must contain {start}"));
    let rest = &sql[from + start.len()..];
    let to = rest
        .find(end)
        .unwrap_or_else(|| panic!("statement must contain {end} after {start}"));
    &rest[..to]
}

/// The type and constraints of `ADD COLUMN <name>` in an `ALTER TABLE`.
fn added_column(sql: &str, name: &str) -> String {
    let prefix = format!("ADD COLUMN {name} ");
    sql.lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix(&prefix))
        .unwrap_or_else(|| panic!("migration must add column {name}"))
        .trim_end_matches(';')
        .to_string()
}

/// Statements in `sql`, ignoring comments, so a test can assert a file is one
/// atomic statement rather than a sequence.
fn statement_count(sql: &str) -> usize {
    executable(sql)
        .split(';')
        .map(str::trim)
        .filter(|statement| !statement.is_empty())
        .count()
}

#[test]
fn the_app_private_key_reference_is_additive_nullable_and_detaching() {
    assert_eq!(
        statement_count(APP_KEY_MIGRATION_SQL),
        1,
        "the App key arrives as one additive column and nothing else"
    );
    assert_eq!(
        APP_KEY_MIGRATION_SQL
            .lines()
            .filter(|line| line.contains("ADD COLUMN"))
            .count(),
        1,
        "the migration adds no column beyond the App key reference"
    );
    let column = added_column(APP_KEY_MIGRATION_SQL, "app_private_key_secret_key");
    assert!(
        column.starts_with("TEXT") && !column.contains("NOT NULL"),
        "a connection without an App key is a normal state: {column}"
    );
    assert!(
        APP_KEY_MIGRATION_SQL
            .contains("REFERENCES context69.internal_secrets(key) ON DELETE SET NULL"),
        "clearing a store row detaches the reference instead of failing"
    );
    // Purpose separation: the token reference is untouched, so the App key has
    // its own column and cannot displace the credential a PAT owns.
    assert!(
        !executable(APP_KEY_MIGRATION_SQL).contains("credential_secret_key")
            && !executable(APP_KEY_MIGRATION_SQL).contains("signing_secret_key"),
        "the migration adds a reference and never moves another secret's column"
    );
    // The applied phase-2 and 3B1 migrations stay immutable.
    for applied in [PHASE2_MIGRATION_SQL, GROUPS_MIGRATION_SQL] {
        assert!(
            !applied.contains("app_private_key_secret_key"),
            "an applied migration must not gain the column it did not have"
        );
    }
    // Every read of a connection reports the reference, so a stored App key is
    // never invisible to the writer that has to repoint it.
    for query in [
        UPSERT_CONNECTION_SQL,
        GET_CONNECTION_SQL,
        LIST_CONNECTIONS_SQL,
        INSERT_CONNECTION_SQL,
    ] {
        assert!(
            query.contains("app_private_key_secret_key"),
            "connection reads must project the App key reference: {}",
            &query[..query.len().min(60)]
        );
    }
}

#[test]
fn secret_rotation_moves_one_reference_and_never_a_lifecycle_column() {
    // Each narrow statement writes one reference column, and none of them can
    // reach a lifecycle column: a rotation must not re-enable a disabled
    // connection nor re-activate a deactivated hook.
    for (sql, column) in [
        (SET_CONNECTION_CREDENTIAL_SQL, "credential_secret_key"),
        (SET_CONNECTION_APP_KEY_SQL, "app_private_key_secret_key"),
        (SET_WEBHOOK_SIGNING_SECRET_SQL, "signing_secret_key"),
    ] {
        let body = executable(sql);
        assert_eq!(statement_count(sql), 1, "a reference move is one statement");
        assert!(
            body.contains(&format!("{column} = $3")),
            "the narrow update must set exactly its own column: {column}"
        );
        for forbidden in ["disabled_at", "active", "provider_kind", "DELETE", "INSERT"] {
            assert!(
                !body.contains(forbidden),
                "a reference move must not touch {forbidden}"
            );
        }
        assert!(
            body.contains("updated_at = now()"),
            "the move still records that the row changed"
        );
    }
    // The broad connection upsert is not a rotation path: it clears the disabled
    // state, and it neither inserts nor assigns the App key, so a metadata save
    // cannot destroy a reference a writer established — while still projecting
    // that reference so the writer can find it.
    let upsert = executable(UPSERT_CONNECTION_SQL);
    assert!(
        upsert.contains("disabled_at = NULL"),
        "the broad upsert still re-enables a connection; that is why it is not reused"
    );
    assert!(
        !upsert.contains("app_private_key_secret_key ="),
        "no rotation may reach the App key through the broad upsert"
    );
    assert!(
        !between(
            &upsert,
            "INSERT INTO context69.git_provider_connections (",
            "\n    )"
        )
        .contains("app_private_key_secret_key"),
        "the broad upsert must not insert the App key"
    );
    assert!(
        !between(&upsert, "ON CONFLICT", "RETURNING").contains("app_private_key_secret_key"),
        "the broad upsert must not assign the App key on conflict"
    );
    assert!(
        upsert.contains("upserted.app_private_key_secret_key"),
        "the upsert still projects the reference a writer established"
    );
}

#[test]
fn the_create_insert_projects_the_app_key_without_assigning_it() {
    let body = executable(INSERT_CONNECTION_SQL);
    assert_eq!(
        statement_count(INSERT_CONNECTION_SQL),
        1,
        "a create is one atomic statement"
    );
    assert!(
        !body.contains("ON CONFLICT"),
        "the create path must not reuse the re-enabling conflict upsert"
    );
    assert!(
        !body.contains("disabled_at =") && !body.contains("app_private_key_secret_key ="),
        "a create sets neither the lifecycle nor the App-key column"
    );
    let columns = between(
        &body,
        "INSERT INTO context69.git_provider_connections (",
        "\n    )",
    );
    assert!(
        !columns.contains("app_private_key_secret_key"),
        "the App key reference is never inserted, only moved by its own statement"
    );
    assert!(
        body.contains("inserted.app_private_key_secret_key"),
        "the create still projects the reference a later writer establishes"
    );
}

#[test]
fn a_reference_update_is_group_confined_through_its_own_ownership() {
    // Connection references filter on the owning group directly.
    for sql in [SET_CONNECTION_CREDENTIAL_SQL, SET_CONNECTION_APP_KEY_SQL] {
        assert!(
            sql.contains("WHERE group_id = $1") && sql.contains("connection_key = $2"),
            "a connection reference update must be confined to its group: {}",
            &sql[sql.len().saturating_sub(120)..]
        );
    }
    // A webhook reference has no group column of its own, so ownership comes
    // from the registration's repository source, as every other webhook
    // operation already does.
    assert!(
        SET_WEBHOOK_SIGNING_SECRET_SQL.contains("context69.git_repository_sources")
            && SET_WEBHOOK_SIGNING_SECRET_SQL.contains("s.group_id = $1")
            && SET_WEBHOOK_SIGNING_SECRET_SQL.contains("s.repository_key = r.repository_key"),
        "a webhook reference update must be scoped through the repository source"
    );
}
