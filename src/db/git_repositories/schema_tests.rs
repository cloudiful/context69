//! Schema invariants the Git persistence layer depends on (issue #681 work unit
//! 3B1).
//!
//! These assert the migration and query-file guarantees this module is built
//! on: mandatory group ownership, per-group repository/ref identity, visibility
//! read only from `context69.groups`, generations carrying metadata without
//! content, and generation completion plus activation being one atomic
//! statement.

const GROUPS_MIGRATION_SQL: &str =
    include_str!("../../../migrations/20261001010156_git_repository_groups_generations.sql");
const COUNTER_MIGRATION_SQL: &str =
    include_str!("../../../migrations/20261001014131_git_repository_generation_counter.sql");
const PHASE2_MIGRATION_SQL: &str =
    include_str!("../../../migrations/20260930204952_git_repository_sources.sql");
const START_GENERATION_SQL: &str =
    include_str!("../../sql/db/git_repository_generations/start_git_repository_generation.sql");
const ACTIVATE_GENERATION_SQL: &str = include_str!(
    "../../sql/db/git_repository_generations/complete_and_activate_git_repository_generation.sql"
);
const FAIL_GENERATION_SQL: &str =
    include_str!("../../sql/db/git_repository_generations/fail_git_repository_generation.sql");
const GET_GENERATION_SQL: &str =
    include_str!("../../sql/db/git_repository_generations/get_git_repository_generation.sql");
const LIST_GENERATIONS_SQL: &str =
    include_str!("../../sql/db/git_repository_generations/list_git_repository_generations.sql");
const GET_ACTIVE_GENERATION_SQL: &str =
    include_str!("../../sql/db/git_repository_generations/get_git_active_generation.sql");
const UPSERT_SOURCE_SQL: &str =
    include_str!("../../sql/db/git_repositories/upsert_git_repository_source.sql");
const GET_SOURCE_SQL: &str =
    include_str!("../../sql/db/git_repositories/get_git_repository_source.sql");
const LIST_SOURCE_SQL: &str =
    include_str!("../../sql/db/git_repositories/list_git_repository_sources.sql");
const CHECKPOINT_SQL: &str =
    include_str!("../../sql/db/git_repositories/update_git_repository_checkpoint.sql");
const UPSERT_CONNECTION_SQL: &str =
    include_str!("../../sql/db/git_repositories/upsert_git_provider_connection.sql");
const INSERT_CONNECTION_SQL: &str =
    include_str!("../../sql/db/git_repositories/insert_git_provider_connection.sql");
const ACQUIRE_CREATION_LOCK_SQL: &str =
    include_str!("../../sql/db/git_repositories/acquire_git_connection_creation_lock.sql");
const GET_CONNECTION_SQL: &str =
    include_str!("../../sql/db/git_repositories/get_git_provider_connection.sql");
const LIST_CONNECTIONS_SQL: &str =
    include_str!("../../sql/db/git_repositories/list_git_provider_connections.sql");
const DISABLE_CONNECTION_SQL: &str =
    include_str!("../../sql/db/git_repositories/disable_git_provider_connection.sql");
const ENABLE_CONNECTION_SQL: &str =
    include_str!("../../sql/db/git_repositories/enable_git_provider_connection.sql");
const WEBHOOK_REGISTRATION_SQL: &str =
    include_str!("../../sql/db/git_repositories/upsert_git_webhook_registration.sql");
const GET_WEBHOOK_REGISTRATION_SQL: &str =
    include_str!("../../sql/db/git_repositories/get_git_webhook_registration.sql");
const INSERT_WEBHOOK_REGISTRATION_SQL: &str =
    include_str!("../../sql/db/git_repositories/insert_git_webhook_registration.sql");
const ACQUIRE_WEBHOOK_CREATION_LOCK_SQL: &str = include_str!(
    "../../sql/db/git_repositories/acquire_git_webhook_registration_creation_lock.sql"
);

/// Body of a `CREATE TABLE IF NOT EXISTS` in `sql`, without the closing paren.
fn table_body<'a>(sql: &'a str, table: &str) -> &'a str {
    let marker = format!("CREATE TABLE IF NOT EXISTS context69.{table} (");
    let start = sql
        .find(&marker)
        .unwrap_or_else(|| panic!("migration must create {table}"));
    let rest = &sql[start + marker.len()..];
    let mut depth = 1_i32;
    for (index, character) in rest.char_indices() {
        match character {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return &rest[..index];
                }
            }
            _ => {}
        }
    }
    panic!("unterminated CREATE TABLE for {table}");
}

fn has_column(body: &str, name: &str) -> bool {
    body.lines().any(|line| {
        line.trim_start()
            .strip_prefix(name)
            .is_some_and(|rest| rest.starts_with(char::is_whitespace))
    })
}

fn assert_columns(table: &str, body: &str, columns: &[&str]) {
    for column in columns {
        assert!(
            has_column(body, column),
            "{table} must define column {column}"
        );
    }
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

/// `sql` with its comment lines removed, so an assertion about what a statement
/// *does* is never satisfied or broken by prose explaining it.
fn code(sql: &str) -> String {
    sql.lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Statements in `sql`, ignoring comments, so a test can assert a file is one
/// atomic statement rather than a sequence.
fn statement_count(sql: &str) -> usize {
    code(sql)
        .split(';')
        .map(str::trim)
        .filter(|statement| !statement.is_empty())
        .count()
}

#[test]
fn group_ownership_is_mandatory_and_never_synthetic() {
    for table in ["git_provider_connections", "git_repository_sources"] {
        assert!(
            GROUPS_MIGRATION_SQL.contains(&format!(
                "ALTER TABLE context69.{table}\n    ADD COLUMN group_id BIGINT NOT NULL \
                 REFERENCES context69.groups(id) ON DELETE CASCADE"
            )),
            "{table} must be owned by exactly one group, cascaded with it"
        );
    }
    assert!(
        GROUPS_MIGRATION_SQL.contains("PRIMARY KEY (group_id, connection_key)"),
        "the provider connection key is group-scoped"
    );
    assert!(
        GROUPS_MIGRATION_SQL.contains("FOREIGN KEY (group_id, connection_key)")
            && GROUPS_MIGRATION_SQL.contains(
                "REFERENCES context69.git_provider_connections (group_id, connection_key)"
            ),
        "a source may only reference a connection of its own group"
    );
    assert!(
        !GROUPS_MIGRATION_SQL.contains("INSERT INTO context69.groups"),
        "ownership must not be satisfied by inventing a shared group"
    );
}

#[test]
fn repository_ref_identity_is_per_group() {
    assert!(
        GROUPS_MIGRATION_SQL.contains("UNIQUE (group_id, canonical_url, target_ref)"),
        "repository ref identity is per group"
    );
    assert!(
        GROUPS_MIGRATION_SQL.contains("DROP CONSTRAINT uq_git_repository_sources_ref"),
        "the globally unique repository ref must be replaced, not kept alongside"
    );
}

#[test]
fn visibility_is_read_from_the_authoritative_groups_join_only() {
    // Queries that project group identity read it from the groups row.
    for query in [
        UPSERT_SOURCE_SQL,
        UPSERT_CONNECTION_SQL,
        INSERT_CONNECTION_SQL,
        CHECKPOINT_SQL,
        GET_SOURCE_SQL,
    ] {
        assert!(
            query.contains("JOIN context69.groups g ON g.id =")
                && query.contains("g.visibility AS group_visibility"),
            "group identity and current visibility come from the groups row: {}",
            &query[..query.len().min(60)]
        );
    }
    // Generations are repository-scoped and project no group copy.
    assert!(
        !table_body(GROUPS_MIGRATION_SQL, "git_repository_generations").contains("visibility"),
        "generations carry no group visibility copy"
    );
    let lower = GROUPS_MIGRATION_SQL.to_ascii_lowercase();
    assert!(
        !lower.contains("add column group_visibility")
            && !lower.contains("add column visibility")
            && !lower.contains("visibility text"),
        "visibility must not be stored on git rows"
    );
    assert!(
        !table_body(PHASE2_MIGRATION_SQL, "git_repository_sources").contains("visibility"),
        "the phase 2 table stored no visibility either"
    );
}

#[test]
fn every_group_scoped_operation_cannot_reach_another_group() {
    // Reads and updates filter on the owning group.
    for query in [
        GET_SOURCE_SQL,
        LIST_SOURCE_SQL,
        CHECKPOINT_SQL,
        ACTIVATE_GENERATION_SQL,
        FAIL_GENERATION_SQL,
        START_GENERATION_SQL,
        GET_GENERATION_SQL,
        LIST_GENERATIONS_SQL,
        GET_ACTIVE_GENERATION_SQL,
        GET_CONNECTION_SQL,
        LIST_CONNECTIONS_SQL,
        DISABLE_CONNECTION_SQL,
        ENABLE_CONNECTION_SQL,
    ] {
        assert!(
            query.contains("group_id = $1") || query.contains("group_id = $2"),
            "query must filter on the owning group: {}",
            &query[..query.len().min(60)]
        );
    }
    // Upserts take the owner as the value they insert and key the conflict on it.
    assert!(
        UPSERT_SOURCE_SQL.contains("ON CONFLICT (group_id, canonical_url, target_ref)"),
        "the source upsert keys its conflict on the owning group"
    );
    assert!(
        UPSERT_CONNECTION_SQL.contains("ON CONFLICT (group_id, connection_key)"),
        "the connection upsert keys its conflict on the owning group"
    );
    // Webhook registrations are scoped through their repository source.
    for query in [
        WEBHOOK_REGISTRATION_SQL,
        GET_WEBHOOK_REGISTRATION_SQL,
        INSERT_WEBHOOK_REGISTRATION_SQL,
    ] {
        assert!(
            query.contains("s.group_id = $1") || query.contains("s.group_id = $2"),
            "webhook registration must be scoped through the repository source: {}",
            &query[..query.len().min(60)]
        );
    }
}

#[test]
fn webhook_registration_create_is_a_create_only_group_and_repository_scoped_insert() {
    assert_eq!(
        statement_count(INSERT_WEBHOOK_REGISTRATION_SQL),
        1,
        "registration creation must be one atomic statement"
    );
    let statement = code(INSERT_WEBHOOK_REGISTRATION_SQL);
    assert!(
        statement.contains("INSERT INTO context69.git_webhook_registrations"),
        "the statement inserts a webhook registration"
    );
    // Create-only: the broad upsert's conflict clause would overwrite a
    // registration another request owns, so a create must have no conflict arm
    // and let the unique violation be reported instead.
    assert!(
        !statement.contains("ON CONFLICT") && !statement.contains("DO UPDATE"),
        "a create-only insert must have no conflict clause"
    );
    // Confined to one repository of one group, and the row is selected from that
    // source rather than named by the caller, so a foreign key inserts nothing.
    assert!(
        statement.contains("s.repository_key = $1")
            && statement.contains("s.group_id = $2")
            && statement.contains("s.repository_key,"),
        "the insert writes only onto a repository this group owns"
    );
    // The sealed reference rides on the same insert, so a registration can never
    // exist without a resolvable reference to the value that was sealed for it.
    assert!(
        statement.contains("signing_secret_key")
            && statement.contains("$7")
            && !statement.contains("internal_secrets"),
        "the reference is a bound value, and the statement never reads the store"
    );
    for forbidden in ["updated_at =", "DELETE", "TRUNCATE"] {
        assert!(
            !statement.contains(forbidden),
            "the create insert must not {forbidden}"
        );
    }
}

#[test]
fn webhook_registration_creation_takes_a_transaction_scoped_advisory_lock() {
    assert_eq!(
        statement_count(ACQUIRE_WEBHOOK_CREATION_LOCK_SQL),
        1,
        "the registration creation lock is one binding statement"
    );
    assert!(
        ACQUIRE_WEBHOOK_CREATION_LOCK_SQL.contains("pg_advisory_xact_lock"),
        "creation must take a transaction-scoped advisory lock, so the end of \
         the create transaction releases it"
    );
    assert!(
        !ACQUIRE_WEBHOOK_CREATION_LOCK_SQL.contains("pg_advisory_lock"),
        "a session-scoped lock would outlive the create transaction"
    );
    assert!(
        ACQUIRE_WEBHOOK_CREATION_LOCK_SQL.contains("$1"),
        "the lock key is a bound parameter, never a literal, so each \
         (group, repository) derives its own lock"
    );
}

#[test]
fn connection_enable_clears_only_the_lifecycle_column_and_never_reads_a_secret() {
    // One statement, scoped to the owning group and the key.
    assert_eq!(
        statement_count(ENABLE_CONNECTION_SQL),
        1,
        "enabling a connection is one atomic statement"
    );
    let statement = code(ENABLE_CONNECTION_SQL);
    assert!(
        statement.contains("group_id = $1") && statement.contains("connection_key = $2"),
        "the enable must match the owning group and the key"
    );
    // Only the lifecycle column moves. A secret column here would let a repeated
    // enable rotate or drop a credential, so its absence is the invariant.
    assert!(
        statement.contains("disabled_at = NULL") && statement.contains("updated_at = now()"),
        "enabling clears the disabled marker and stamps freshness, nothing else"
    );
    for forbidden in [
        "internal_secrets",
        "credential_secret_key",
        "webhook_secret_key",
        "app_private_key_secret_key",
    ] {
        assert!(
            !statement.contains(forbidden),
            "the enable statement must not touch {forbidden}"
        );
    }
    // A guard on `disabled_at` would make a repeated enable a no-match conflict;
    // the whole point is that the predicate is the group and the key only.
    assert!(
        !statement.contains("IS NOT NULL"),
        "an already-enabled connection must still match"
    );
}

#[test]
fn connection_create_is_a_create_only_group_scoped_insert() {
    assert_eq!(
        statement_count(INSERT_CONNECTION_SQL),
        1,
        "creation must be one atomic statement"
    );
    assert!(
        INSERT_CONNECTION_SQL.contains("INSERT INTO context69.git_provider_connections"),
        "the statement inserts a provider connection"
    );
    assert!(
        !INSERT_CONNECTION_SQL.contains("ON CONFLICT"),
        "a create-only insert must have no conflict clause, so a duplicate key \
         is reported instead of overwriting or re-enabling a connection"
    );
    assert!(
        INSERT_CONNECTION_SQL.contains("VALUES ($1,"),
        "the owning group is the first insert value"
    );
    assert!(
        INSERT_CONNECTION_SQL.contains("JOIN context69.groups g ON g.id = inserted.group_id"),
        "the returned group identity comes from the owning groups row"
    );
    // A new row is enabled by the schema default: neither lifecycle column is
    // assigned, and the App key reference is projected but never inserted.
    for forbidden in [
        "disabled_at =",
        "updated_at =",
        "app_private_key_secret_key =",
    ] {
        assert!(
            !INSERT_CONNECTION_SQL.contains(forbidden),
            "the create insert must not assign {forbidden}"
        );
    }
    let columns = &INSERT_CONNECTION_SQL[INSERT_CONNECTION_SQL
        .find("INSERT INTO context69.git_provider_connections (")
        .expect("insert column list")..];
    let columns = &columns[..columns.find("\n    )").expect("end of column list")];
    assert!(
        !columns.contains("app_private_key_secret_key"),
        "the insert column list must not name the App key reference"
    );
    assert!(
        INSERT_CONNECTION_SQL.contains("inserted.app_private_key_secret_key"),
        "the statement still projects the App key reference a later writer sets"
    );
}

#[test]
fn connection_creation_takes_a_transaction_scoped_advisory_lock() {
    assert_eq!(
        statement_count(ACQUIRE_CREATION_LOCK_SQL),
        1,
        "the creation lock is one binding statement"
    );
    assert!(
        ACQUIRE_CREATION_LOCK_SQL.contains("pg_advisory_xact_lock"),
        "creation must take a transaction-scoped advisory lock, so the end of \
         the create transaction releases it"
    );
    assert!(
        ACQUIRE_CREATION_LOCK_SQL.contains("$1"),
        "the lock key is a bound parameter, never a literal, so each \
         (group, connection key) derives its own lock"
    );
    assert!(
        !ACQUIRE_CREATION_LOCK_SQL.contains("pg_advisory_lock"),
        "a session-scoped lock would outlive the create transaction"
    );
}

#[test]
fn generations_are_metadata_without_content() {
    assert_columns(
        "git_repository_generations",
        table_body(GROUPS_MIGRATION_SQL, "git_repository_generations"),
        &[
            "generation_key",
            "repository_key",
            "generation_number",
            "ref_name",
            "commit_sha",
            "index_profile",
            "status",
            "file_count",
            "excluded_file_count",
            "total_bytes",
            "error_code",
            "started_at",
            "completed_at",
        ],
    );
    let body = table_body(GROUPS_MIGRATION_SQL, "git_repository_generations");
    for forbidden in [
        "content",
        "body",
        "payload",
        "chunk",
        "embedding",
        "vector",
        "lines",
        "checksum",
    ] {
        assert!(
            !body.to_ascii_lowercase().contains(forbidden),
            "generations must stay metadata only, found {forbidden}"
        );
    }
    assert!(
        body.contains("status IN ('building', 'ready', 'failed', 'superseded')")
            && body.contains("generation_number > 0")
            && body.contains(
                "CHECK (file_count >= 0 AND excluded_file_count >= 0 AND total_bytes >= 0)"
            ),
        "generation state, numbering, and counters are constrained"
    );
    assert!(
        body.contains("CHECK ((status = 'building') = (completed_at IS NULL))"),
        "a generation is building exactly while it is incomplete"
    );
}

#[test]
fn active_generation_pointer_is_confined_to_its_repository() {
    assert_columns(
        "git_repository_active_generations",
        table_body(GROUPS_MIGRATION_SQL, "git_repository_active_generations"),
        &["repository_key", "generation_key", "activated_at"],
    );
    let body = table_body(GROUPS_MIGRATION_SQL, "git_repository_active_generations");
    assert!(
        body.contains("FOREIGN KEY (repository_key, generation_key)")
            && body.contains(
                "REFERENCES context69.git_repository_generations (repository_key, generation_key)"
            ),
        "the pointer may only reference a generation of its own repository"
    );
    assert!(
        table_body(GROUPS_MIGRATION_SQL, "git_repository_generations")
            .contains("UNIQUE (repository_key, generation_key)"),
        "the composite reference target must be unique"
    );
}

#[test]
fn generation_completion_and_activation_is_one_atomic_statement() {
    assert_eq!(
        statement_count(ACTIVATE_GENERATION_SQL),
        1,
        "completion, supersession, and the pointer move must be one statement"
    );
    for fragment in [
        "SET status = 'ready'",
        "completed_at = now()",
        "status = 'superseded'",
        "INSERT INTO context69.git_repository_active_generations",
        "ON CONFLICT (repository_key) DO UPDATE",
        "g.status = 'building'",
    ] {
        assert!(
            ACTIVATE_GENERATION_SQL.contains(fragment),
            "activation must include {fragment}"
        );
    }
    assert!(
        !ACTIVATE_GENERATION_SQL.contains("UPDATE context69.git_repository_sources"),
        "the source status projection stays with the checkpoint operation"
    );
    assert_eq!(
        statement_count(FAIL_GENERATION_SQL),
        1,
        "a failure records a bounded code in one statement"
    );
    assert!(
        ACTIVATE_GENERATION_SQL.contains("file_count = $4")
            && ACTIVATE_GENERATION_SQL.contains("excluded_file_count = $5")
            && ACTIVATE_GENERATION_SQL.contains("total_bytes = $6"),
        "coverage counters land with the completion, not later"
    );
}

#[test]
fn generation_number_is_allocated_from_a_committed_repository_counter() {
    // The number comes from the repository's own counter, so the statement
    // cannot derive it from a read that predates a concurrent commit.
    assert_eq!(
        statement_count(START_GENERATION_SQL),
        1,
        "allocating and inserting the generation is one statement"
    );
    let lower = START_GENERATION_SQL.to_ascii_lowercase();
    for forbidden in [
        "max(generation_number)",
        "select coalesce",
        "order by generation_number",
    ] {
        assert!(
            !lower.contains(forbidden),
            "the next number must not be derived from a read of the generations \
             table, which cannot see a concurrent commit: found {forbidden}"
        );
    }
    for fragment in [
        "UPDATE context69.git_repository_sources",
        "SET generation_counter = generation_counter + 1",
        "RETURNING repository_key, generation_counter",
        "allocated.generation_counter",
    ] {
        assert!(
            START_GENERATION_SQL.contains(fragment),
            "the start statement must include {fragment}"
        );
    }
    assert!(
        START_GENERATION_SQL.find("UPDATE context69.git_repository_sources")
            < START_GENERATION_SQL.find("INSERT INTO context69.git_repository_generations"),
        "the counter is incremented before the generation row is inserted"
    );
}

#[test]
fn generation_counter_is_backfilled_and_cannot_go_negative() {
    assert_eq!(
        added_column(COUNTER_MIGRATION_SQL, "generation_counter"),
        "BIGINT NOT NULL DEFAULT 0",
        "the counter defaults to zero so a new repository starts at generation 1"
    );
    assert!(
        COUNTER_MIGRATION_SQL
            .contains("ADD CONSTRAINT chk_git_repository_sources_generation_counter")
            && COUNTER_MIGRATION_SQL.contains("CHECK (generation_counter >= 0)"),
        "the counter is constrained so an allocation can never produce a \
         non-positive generation number"
    );
    // Numbering continues from the stored history instead of restarting at 1.
    assert!(
        COUNTER_MIGRATION_SQL.contains("SELECT MAX(g.generation_number)")
            && COUNTER_MIGRATION_SQL.contains(
                "FROM context69.git_repository_generations g\n        WHERE g.repository_key = s.repository_key"
            ),
        "existing repositories resume from their highest stored generation"
    );
    assert!(
        !GROUPS_MIGRATION_SQL.contains("generation_counter"),
        "the counter is added by its own additive migration, so the applied \
         3B1 migration stays immutable"
    );
}

#[test]
fn secrets_stay_internal_secret_references_and_none_are_added() {
    let lower = GROUPS_MIGRATION_SQL.to_ascii_lowercase();
    for forbidden in ["access_token", "password", "private_key"] {
        assert!(
            !lower.contains(forbidden),
            "migration must not persist plaintext {forbidden}"
        );
    }
    for line in GROUPS_MIGRATION_SQL
        .lines()
        .map(str::trim)
        .filter(|line| line.contains("ADD COLUMN"))
    {
        let column = line
            .split("ADD COLUMN")
            .nth(1)
            .expect("column after ADD COLUMN")
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        assert!(
            !column.contains("secret") && !column.contains("token") && !column.contains("password"),
            "3B1 adds ownership and generation columns only, found {column}"
        );
    }
    for table in ["git_provider_connections", "git_webhook_registrations"] {
        let body = table_body(PHASE2_MIGRATION_SQL, table);
        let secret_lines = body
            .lines()
            .map(str::trim)
            .filter(|line| line.contains("_secret_key"))
            .collect::<Vec<_>>();
        assert!(
            !secret_lines.is_empty(),
            "{table} must still reference the internal secret store"
        );
        for line in secret_lines {
            assert!(
                line.contains("REFERENCES context69.internal_secrets(key)")
                    && !line.contains("NOT NULL"),
                "{table} secret references must stay optional internal keys: {line}"
            );
        }
    }
}
