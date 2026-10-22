//! Database-gated checks for the explicit library bootstrap and for the
//! linkage columns that connect a domain item to a library execution segment
//! (issue #1020 P1).
//!
//! The `cloudiful-tasks` library owns one schema inside the application
//! database. That schema is created only by an explicit bootstrap, it is never
//! created by connecting the application, and the runner that resets the
//! application schema never touches it. These tests cover that contract
//! against the configured test database: the schema is missing until it is
//! bootstrapped, the bootstrap is idempotent, a schema that only looks like
//! the library's own is refused, and the read-only preflight recognizes the
//! ledger the library owns while still refusing an unknown foreign ledger.
//!
//! The checks that need the library namespace to be free run inside one
//! transaction that is rolled back at the end, so the schema the library owns
//! is exactly as it was before the check. The linkage columns are read from a
//! rolled-back fixture, so no row survives the test either.

use context69_db_schema::{LIBRARY_SCHEMA, bootstrap_library_schema};
use context69_test_support::{
    accessor::database_url,
    config::Endpoint,
    runner::preflight::{self, Refusal},
};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tokio::sync::Mutex;
use uuid::Uuid;

/// The library schema is one shared object in the test database, so the checks
/// that move it aside run one at a time.
static LIBRARY_SCHEMA_LOCK: Mutex<()> = Mutex::const_new(());

/// Name the library schema takes while a check inspects the namespace.
const MOVED_ASIDE_SCHEMA: &str = "context69_test_library_owned";

/// Name of the schema a foreign-ledger check owns.
const FOREIGN_LEDGER_SCHEMA: &str = "context69_test_foreign_ledger";

/// The configured test database.
fn endpoint() -> Endpoint {
    Endpoint::parse(&database_url()).expect("the configured test database is a usable URL")
}

/// A pool that carries the runner's application name, so the preflight does
/// not read this process as another backend attached to the database.
async fn pool() -> PgPool {
    let options = preflight::connect_options(&endpoint())
        .expect("the runner's connection options for the test database");
    PgPool::connect_with(options)
        .await
        .expect("the test database accepts a connection")
}

/// A pool whose single connection is one open transaction, so every statement
/// on it is undone when the pool is closed.
async fn transactional_pool() -> PgPool {
    PgPoolOptions::new()
        .max_connections(1)
        .after_connect(|connection, _metadata| {
            Box::pin(async move {
                sqlx::raw_sql("BEGIN").execute(&mut *connection).await?;
                Ok(())
            })
        })
        .connect(&database_url())
        .await
        .expect("the test database accepts a connection")
}

/// Whether the library schema exists in the database right now.
async fn library_schema_exists(pool: &PgPool) -> bool {
    sqlx::query_file_scalar!(
        "tests/sql/task_execution/library_schema_exists.sql",
        LIBRARY_SCHEMA
    )
    .fetch_one(pool)
    .await
    .expect("the library schema is observable")
}

/// Remove the schema an interrupted check may have left behind.
///
/// The name belongs to this module alone, so removing it never reaches the
/// schema the library owns.
async fn drop_moved_aside_schema(pool: &PgPool) {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "DROP SCHEMA IF EXISTS {MOVED_ASIDE_SCHEMA} CASCADE"
    )))
    .execute(pool)
    .await
    .expect("the leftover schema is removed");
}

/// Free the library namespace for a check that runs inside a transaction.
///
/// The schema the library owns is moved aside inside that transaction, so the
/// rollback restores it under its own name.
async fn move_library_schema_aside(pool: &PgPool) {
    if library_schema_exists(pool).await {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "ALTER SCHEMA {LIBRARY_SCHEMA} RENAME TO {MOVED_ASIDE_SCHEMA}"
        )))
        .execute(pool)
        .await
        .expect("the library schema moves aside inside the transaction");
    }
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_missing_library_schema_is_refused_by_the_read_only_check() {
    let _guard = LIBRARY_SCHEMA_LOCK.lock().await;
    let pool = pool().await;
    drop_moved_aside_schema(&pool).await;

    let transactional = transactional_pool().await;
    move_library_schema_aside(&transactional).await;

    let missing = cloudiful_tasks::check_schema(&transactional)
        .await
        .expect_err("a library schema that does not exist is refused");
    assert!(
        matches!(missing, cloudiful_tasks::Error::SchemaMissing { .. }),
        "the refusal names the missing schema, got {missing:?}"
    );

    // The rollback restores the schema the library owns under its own name.
    transactional.close().await;
    assert!(
        library_schema_exists(&pool).await,
        "the schema the library owns is back under its own name"
    );
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn the_explicit_bootstrap_creates_the_library_schema_and_is_idempotent() {
    let _guard = LIBRARY_SCHEMA_LOCK.lock().await;
    let pool = pool().await;
    drop_moved_aside_schema(&pool).await;

    let first = bootstrap_library_schema(&pool)
        .await
        .expect("the library schema is bootstrapped");
    assert!(
        !first.info().instance_uuid().is_nil(),
        "the schema instance carries an identity"
    );
    assert!(
        !first.info().applied_versions().is_empty(),
        "the applied migration is recorded"
    );

    let second = bootstrap_library_schema(&pool)
        .await
        .expect("a second bootstrap succeeds");
    assert!(!second.applied(), "the second bootstrap applies nothing");
    assert_eq!(
        second.info().instance_uuid(),
        first.info().instance_uuid(),
        "the schema instance keeps its identity"
    );
    assert_eq!(
        second.info().applied_versions(),
        first.info().applied_versions(),
        "the applied migrations stay the same"
    );
    assert_eq!(
        second.info().protocol_version(),
        first.info().protocol_version(),
        "the protocol version stays the same"
    );
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_schema_that_only_looks_like_the_librarys_own_is_refused() {
    let _guard = LIBRARY_SCHEMA_LOCK.lock().await;
    let pool = pool().await;
    drop_moved_aside_schema(&pool).await;

    let transactional = transactional_pool().await;
    move_library_schema_aside(&transactional).await;

    sqlx::query(sqlx::AssertSqlSafe(format!(
        "CREATE SCHEMA {LIBRARY_SCHEMA}"
    )))
    .execute(&transactional)
    .await
    .expect("the lookalike schema is created");
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "CREATE TABLE {LIBRARY_SCHEMA}._sqlx_migrations (
             version BIGINT PRIMARY KEY,
             description TEXT NOT NULL,
             installed_on TIMESTAMPTZ NOT NULL DEFAULT now(),
             success BOOLEAN NOT NULL,
             checksum BYTEA NOT NULL,
             execution_time BIGINT NOT NULL
         )"
    )))
    .execute(&transactional)
    .await
    .expect("the lookalike ledger is created");

    let refused = cloudiful_tasks::check_schema(&transactional)
        .await
        .expect_err("a schema the library does not own is refused");
    assert!(
        matches!(refused, cloudiful_tasks::Error::ForeignSchema { .. }),
        "the refusal names the foreign schema, got {refused:?}"
    );

    let bootstrap = bootstrap_library_schema(&transactional)
        .await
        .expect_err("the bootstrap refuses a schema it does not own");
    assert!(
        matches!(bootstrap, cloudiful_tasks::Error::ForeignSchema { .. }),
        "the bootstrap refuses the lookalike, got {bootstrap:?}"
    );

    // The lookalike and the moved schema are undone with the transaction, so
    // the schema the library owns is back under its own name.
    transactional.close().await;
    assert!(
        library_schema_exists(&pool).await,
        "the schema the library owns is back under its own name"
    );
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn the_preflight_recognises_the_ledger_the_library_owns() {
    let _guard = LIBRARY_SCHEMA_LOCK.lock().await;
    let pool = pool().await;
    drop_moved_aside_schema(&pool).await;
    bootstrap_library_schema(&pool)
        .await
        .expect("the library schema is bootstrapped");
    let endpoint = endpoint();

    let observation = preflight::collect(&endpoint)
        .await
        .expect("the preflight reads the catalog");
    assert!(
        !observation
            .ledgers
            .iter()
            .any(|ledger| ledger.namespace == LIBRARY_SCHEMA),
        "the ledger of the library is not one of the runner's ledgers"
    );

    let decision = preflight::decide_reset(&observation, &endpoint);
    assert!(
        !decision
            .refusals
            .contains(&Refusal::AmbiguousLedgerNamespace),
        "the library ledger is not an ambiguity, refusals: {:?}",
        decision.refusals
    );
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn an_unknown_foreign_ledger_is_refused_by_the_preflight() {
    let _guard = LIBRARY_SCHEMA_LOCK.lock().await;
    let pool = pool().await;
    drop_moved_aside_schema(&pool).await;
    bootstrap_library_schema(&pool)
        .await
        .expect("the library schema is bootstrapped");
    let endpoint = endpoint();

    sqlx::query(sqlx::AssertSqlSafe(format!(
        "CREATE SCHEMA {FOREIGN_LEDGER_SCHEMA}"
    )))
    .execute(&pool)
    .await
    .expect("the foreign ledger schema is created");
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "CREATE TABLE {FOREIGN_LEDGER_SCHEMA}._sqlx_migrations (
             version BIGINT PRIMARY KEY,
             description TEXT NOT NULL,
             installed_on TIMESTAMPTZ NOT NULL DEFAULT now(),
             success BOOLEAN NOT NULL,
             checksum BYTEA NOT NULL,
             execution_time BIGINT NOT NULL
         )"
    )))
    .execute(&pool)
    .await
    .expect("the foreign ledger is created");

    let observation = preflight::collect(&endpoint)
        .await
        .expect("the preflight reads the catalog");
    let decision = preflight::decide_reset(&observation, &endpoint);

    sqlx::query(sqlx::AssertSqlSafe(format!(
        "DROP SCHEMA {FOREIGN_LEDGER_SCHEMA} CASCADE"
    )))
    .execute(&pool)
    .await
    .expect("the foreign ledger schema is removed again");

    assert!(
        observation
            .ledgers
            .iter()
            .any(|ledger| ledger.namespace == FOREIGN_LEDGER_SCHEMA),
        "the foreign ledger is observed"
    );
    assert!(
        decision
            .refusals
            .contains(&Refusal::AmbiguousLedgerNamespace),
        "an unknown foreign ledger stays a refusal, refusals: {:?}",
        decision.refusals
    );
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn the_linkage_columns_keep_the_defaults_of_the_rows_that_predate_them() {
    let pool = pool().await;
    let mut transaction = pool
        .begin()
        .await
        .expect("the fixture starts a transaction");

    let login_name = format!("p1-linkage-{}", Uuid::new_v4());
    sqlx::query_file!("tests/sql/task_execution/seed_user_mirror.sql", &login_name)
        .execute(&mut *transaction)
        .await
        .expect("the synthetic user is inserted");
    let user_id: i64 = sqlx::query_file_scalar!(
        "tests/sql/task_execution/read_user_id_by_login.sql",
        &login_name
    )
    .fetch_one(&mut *transaction)
    .await
    .expect("the synthetic user is readable");

    let task_id = Uuid::new_v4();
    sqlx::query_file!(
        "tests/sql/task_execution/seed_task_row.sql",
        task_id,
        user_id,
        "p1_linkage_fixture"
    )
    .execute(&mut *transaction)
    .await
    .expect("the synthetic task is inserted");

    let item_id = Uuid::new_v4();
    sqlx::query_file!(
        "tests/sql/task_execution/seed_task_item.sql",
        item_id,
        task_id
    )
    .execute(&mut *transaction)
    .await
    .expect("the synthetic item is inserted");

    let attempt_id: i64 = sqlx::query_file_scalar!(
        "tests/sql/task_execution/seed_task_attempt.sql",
        task_id,
        item_id
    )
    .fetch_one(&mut *transaction)
    .await
    .expect("the synthetic attempt is inserted");

    let item = sqlx::query_file!(
        "tests/sql/task_execution/read_item_segment_columns.sql",
        item_id
    )
    .fetch_one(&mut *transaction)
    .await
    .expect("the linkage columns of the item are readable");
    let execution_task_id: Option<Uuid> = item.execution_task_id;
    let execution_generation: i64 = item.execution_generation;
    assert!(
        execution_task_id.is_none(),
        "an item that predates the library keeps no execution task"
    );
    assert_eq!(
        execution_generation, 0,
        "an item that predates the library keeps generation zero"
    );

    let attempt = sqlx::query_file!(
        "tests/sql/task_execution/read_attempt_segment_columns.sql",
        attempt_id
    )
    .fetch_one(&mut *transaction)
    .await
    .expect("the linkage columns of the attempt are readable");
    let attempt_task_id: Option<Uuid> = attempt.execution_task_id;
    let attempt_attempt_id: Option<Uuid> = attempt.execution_attempt_id;
    assert!(
        attempt_task_id.is_none(),
        "a domain attempt that predates the library keeps no execution task"
    );
    assert!(
        attempt_attempt_id.is_none(),
        "a domain attempt that predates the library keeps no execution attempt"
    );

    transaction
        .rollback()
        .await
        .expect("the fixture leaves no row behind");

    let columns = sqlx::query_file!("tests/sql/task_execution/list_linkage_columns.sql")
        .fetch_all(&pool)
        .await
        .expect("the linkage columns are observable");
    assert_eq!(
        columns.len(),
        4,
        "both linkage columns exist on both relations"
    );
    for column in columns {
        let table_name = column
            .table_name
            .expect("information_schema names every column of the owned tables");
        let column_name = column
            .column_name
            .expect("information_schema names every column of the owned tables");
        let nullable = column
            .is_nullable
            .expect("information_schema reports the nullability of every column");
        let default: Option<String> = column.column_default;
        if column_name == "execution_generation" {
            assert_eq!(nullable, "NO", "the generation is never null");
            assert_eq!(
                default.as_deref(),
                Some("0"),
                "the generation starts at zero"
            );
        } else {
            assert_eq!(nullable, "YES", "{table_name}.{column_name} is nullable");
            assert!(
                default.is_none(),
                "{table_name}.{column_name} has no default"
            );
        }
    }
}
