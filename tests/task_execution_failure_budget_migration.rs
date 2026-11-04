//! Rollback-only verification of the immutable execution-failure-budget
//! migration (issue 1172 P3V).
//!
//! The ordinary suite runs against a schema the runner already migrated, so an
//! empty-schema migration can never prove how existing history is backfilled.
//! This test recreates the exact compiled migration inside one transaction over
//! realistic pre-migration rows, observes the result, and then rolls the whole
//! fixture back. It never edits the migration, never writes the migration
//! ledger, and never leaves a fixture row, column or function behind.

use std::collections::HashSet;

use context69::db::{Database, MIGRATOR};
use serde_json::Value;
use sqlx::{Acquire, PgConnection, Postgres, raw_sql};
use uuid::Uuid;

const MIGRATION_VERSION: i64 = 20261103090220;
const MIGRATION_SQL: &str =
    include_str!("../migrations/20261103090220_task_execution_failure_budget.sql");
const CONFIGURE_TIMEOUTS: &str =
    include_str!("task_execution_failure_budget_migration/configure_timeouts.sql");
const DROP_FAILURE_BUDGET_COLUMN: &str =
    include_str!("task_execution_failure_budget_migration/drop_failure_budget_column.sql");

/// ordinal, status, seeded claim ordinal, seeded failure budget, migrated claim
/// ordinal, migrated failure budget.
const MATRIX: [(i64, &str, i64, i64, i64, i64); 7] = [
    (1, "queued", 5, 1, 5, 5),
    (2, "waiting", 5, 2, 5, 5),
    (3, "queued", 4, 3, 4, 0),
    (4, "running", 5, 4, 5, 0),
    (5, "failed", 5, 5, 5, 0),
    (6, "succeeded", 5, 0, 5, 0),
    (7, "queued", 2, 2, 7, 0),
];

#[derive(Debug, sqlx::FromRow)]
struct Snapshot {
    user_exists: bool,
    task_exists: bool,
    application_ledger_count: i64,
    application_ledger_schemas: Value,
    ledger_schema: Option<String>,
    ledger_version: Option<i64>,
    ledger_checksum: Option<Vec<u8>>,
    ledger_success: Option<bool>,
    ledger_rows: Value,
    fixture_items: Value,
    fixture_history: Value,
    all_items_digest: String,
    column_exists: bool,
    column_not_null: Option<bool>,
    column_type: Option<String>,
    column_default: Option<String>,
    column_check: Option<String>,
    notify_function: String,
}

#[derive(Debug)]
enum GuardedError {
    Database(sqlx::Error),
    Rollback(sqlx::Error),
}

impl From<sqlx::Error> for GuardedError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

struct Guarded {
    user_id: i64,
    pre: Snapshot,
    post: Snapshot,
}

async fn read_snapshot(
    connection: &mut PgConnection,
    task_id: Uuid,
    user_id: i64,
) -> Result<Snapshot, sqlx::Error> {
    sqlx::query_file_as!(
        Snapshot,
        "tests/task_execution_failure_budget_migration/read_snapshot.sql",
        task_id,
        user_id,
        MIGRATION_VERSION
    )
    .fetch_one(connection)
    .await
}

/// Everything between `BEGIN` and the explicit rollback.
///
/// The only statements after the column drop are the migration itself: no
/// `task_items` read or write may happen while the recreated notification
/// function references the absent column.
async fn observe_inside_transaction(
    transaction: &mut sqlx::Transaction<'_, Postgres>,
    task_id: Uuid,
) -> Result<Guarded, sqlx::Error> {
    raw_sql(CONFIGURE_TIMEOUTS)
        .execute(&mut **transaction)
        .await?;
    let user_id: i64 = sqlx::query_file_scalar!(
        "tests/task_execution_failure_budget_migration/seed_fixture.sql",
        task_id
    )
    .fetch_one(&mut **transaction)
    .await?;
    let pre = read_snapshot(&mut **transaction, task_id, user_id).await?;

    raw_sql(DROP_FAILURE_BUDGET_COLUMN)
        .execute(&mut **transaction)
        .await?;
    raw_sql(MIGRATION_SQL).execute(&mut **transaction).await?;

    let post = read_snapshot(&mut **transaction, task_id, user_id).await?;
    Ok(Guarded { user_id, pre, post })
}

/// Run the fixture transaction and always roll it back, on success and on
/// every error path, before any observation is asserted on.
async fn guarded_fixture(
    connection: &mut PgConnection,
    task_id: Uuid,
) -> Result<Guarded, GuardedError> {
    let mut transaction = connection.begin().await?;
    match observe_inside_transaction(&mut transaction, task_id).await {
        Ok(observation) => {
            transaction
                .rollback()
                .await
                .map_err(GuardedError::Rollback)?;
            Ok(observation)
        }
        Err(error) => match transaction.rollback().await {
            Ok(()) => Err(GuardedError::Database(error)),
            Err(rollback) => Err(GuardedError::Rollback(rollback)),
        },
    }
}

fn fixture_items(snapshot: &Snapshot) -> &Vec<Value> {
    snapshot
        .fixture_items
        .as_array()
        .expect("fixture items are a JSON array")
}

fn item_at<'a>(snapshot: &'a Snapshot, ordinal: i64) -> &'a Value {
    fixture_items(snapshot)
        .iter()
        .find(|row| row.get("ordinal").and_then(Value::as_i64) == Some(ordinal))
        .unwrap_or_else(|| panic!("the fixture must seed item ordinal {ordinal}"))
}

fn text_field<'a>(row: &'a Value, name: &str) -> &'a str {
    row.get(name)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("the fixture row must carry a string {name}"))
}

fn integer_field(row: &Value, name: &str) -> i64 {
    row.get(name)
        .and_then(Value::as_i64)
        .unwrap_or_else(|| panic!("the fixture row must carry an integer {name}"))
}

fn masked_row(row: &Value) -> Value {
    let mut masked = row.clone();
    if let Some(object) = masked.as_object_mut() {
        object.remove("execution_failure_count");
        object.remove("attempt_count");
    }
    masked
}

fn assert_application_ledger(snapshot: &Snapshot, migration: &sqlx::migrate::Migration) {
    assert_eq!(
        snapshot.application_ledger_count, 1,
        "exactly one application migration ledger must be visible"
    );
    let schemas = snapshot
        .application_ledger_schemas
        .as_array()
        .expect("ledger schemas are a JSON array");
    assert_eq!(
        schemas.len(),
        1,
        "the application ledger must be unambiguous"
    );
    assert_eq!(
        schemas[0].as_str(),
        snapshot.ledger_schema.as_deref(),
        "the ledger must be the one the connection's search path resolves"
    );
    assert_eq!(
        snapshot.ledger_version,
        Some(migration.version),
        "the applied ledger row must be the migration under test"
    );
    assert_eq!(
        snapshot.ledger_success,
        Some(true),
        "the migration under test must be recorded as applied"
    );
    assert_eq!(
        snapshot.ledger_checksum.as_deref(),
        Some(migration.checksum.as_ref()),
        "the applied checksum must equal the compiled migration checksum"
    );
}

fn assert_failure_budget_column(snapshot: &Snapshot) {
    assert!(
        snapshot.column_exists,
        "the failure budget column must exist"
    );
    assert_eq!(
        snapshot.column_not_null,
        Some(true),
        "the failure budget column must be NOT NULL"
    );
    assert_eq!(
        snapshot.column_type.as_deref(),
        Some("integer"),
        "the failure budget column must be an integer"
    );
    assert_eq!(
        snapshot.column_default.as_deref(),
        Some("0"),
        "the failure budget column must default to zero"
    );
    let check = snapshot
        .column_check
        .as_deref()
        .expect("the failure budget column must carry a check constraint");
    assert!(
        check.contains("execution_failure_count"),
        "the check must cover the failure budget column: {check}"
    );
    assert!(
        check.contains(">= 0") && check.contains("<= 5"),
        "the check must bound the failure budget to 0..5: {check}"
    );
}

fn assert_schema_restored(
    baseline: &Snapshot,
    after: &Snapshot,
    migration: &sqlx::migrate::Migration,
) {
    assert_eq!(after.column_exists, baseline.column_exists);
    assert_eq!(after.column_not_null, baseline.column_not_null);
    assert_eq!(after.column_type, baseline.column_type);
    assert_eq!(after.column_default, baseline.column_default);
    assert_eq!(after.column_check, baseline.column_check);
    assert_eq!(
        after.notify_function, baseline.notify_function,
        "the rollback must restore the migrated notification function"
    );
    assert_eq!(
        after.all_items_digest, baseline.all_items_digest,
        "the rollback must restore every pre-existing item row"
    );
    assert_application_ledger(after, migration);
    assert_eq!(
        after.application_ledger_schemas, baseline.application_ledger_schemas,
        "the rollback must leave the ledger location unchanged"
    );
    assert_eq!(
        after.ledger_rows, baseline.ledger_rows,
        "the rollback must leave every application ledger row unchanged"
    );
    assert_eq!(after.ledger_schema, baseline.ledger_schema);
    assert!(!after.task_exists, "the fixture task must be gone");
    assert!(!after.user_exists, "the fixture user must be gone");
    assert!(
        after
            .fixture_items
            .as_array()
            .expect("items array")
            .is_empty(),
        "the rollback must remove every fixture item"
    );
    assert!(
        after
            .fixture_history
            .as_array()
            .expect("history array")
            .is_empty(),
        "the rollback must remove every fixture history row"
    );
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn the_failure_budget_migration_backfills_historical_rows_and_rolls_back() {
    let url = context69_test_support::accessor::database_url();
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    let mut connection = db.pool().acquire().await.expect("acquire a connection");

    let migration = MIGRATOR
        .iter()
        .find(|migration| migration.version == MIGRATION_VERSION)
        .expect("the compiled migrator embeds the failure-budget migration");
    assert_eq!(
        migration.sql.as_ref(),
        MIGRATION_SQL,
        "the compiled migration must be the immutable migration file byte for byte"
    );

    let task_id = Uuid::new_v4();
    let baseline = read_snapshot(&mut connection, task_id, 0)
        .await
        .expect("read the pre-fixture baseline");
    assert_failure_budget_column(&baseline);
    assert_application_ledger(&baseline, migration);

    let guarded = match guarded_fixture(&mut connection, task_id).await {
        Ok(guarded) => guarded,
        Err(GuardedError::Database(error)) => {
            panic!("the rollback-only migration fixture failed: {error}")
        }
        Err(GuardedError::Rollback(error)) => {
            let _ = connection.close().await;
            panic!("the rollback-only migration fixture could not roll back: {error}")
        }
    };

    let after = read_snapshot(&mut connection, task_id, guarded.user_id)
        .await
        .expect("read the post-rollback snapshot");

    assert_failure_budget_column(&guarded.post);
    assert_application_ledger(&guarded.post, migration);
    assert_eq!(
        guarded.pre.notify_function, baseline.notify_function,
        "seeding the fixture must not change the notification function"
    );
    assert_eq!(
        guarded.post.notify_function, baseline.notify_function,
        "the shipped migration must recreate the same notification function"
    );
    assert_eq!(
        guarded.pre.ledger_rows, baseline.ledger_rows,
        "seeding the fixture must not touch the migration ledger"
    );
    assert_eq!(
        guarded.post.ledger_rows, baseline.ledger_rows,
        "the transaction must not write the migration ledger"
    );

    assert_eq!(fixture_items(&guarded.pre).len(), MATRIX.len());

    for (ordinal, status, seeded_attempt, seeded_budget, migrated_attempt, migrated_budget) in
        MATRIX
    {
        let pre = item_at(&guarded.pre, ordinal);
        assert_eq!(text_field(pre, "status"), status);
        assert_eq!(integer_field(pre, "attempt_count"), seeded_attempt);
        assert_eq!(integer_field(pre, "execution_failure_count"), seeded_budget);

        let post = item_at(&guarded.post, ordinal);
        assert_eq!(
            text_field(post, "status"),
            status,
            "ordinal {ordinal} must keep its status"
        );
        assert_eq!(
            integer_field(post, "attempt_count"),
            migrated_attempt,
            "ordinal {ordinal} must normalize its claim ordinal"
        );
        assert_eq!(
            integer_field(post, "execution_failure_count"),
            migrated_budget,
            "ordinal {ordinal} must derive its failure budget from the old row"
        );
    }

    let pre_masked = fixture_items(&guarded.pre)
        .iter()
        .map(masked_row)
        .collect::<Vec<_>>();
    let post_masked = fixture_items(&guarded.post)
        .iter()
        .map(masked_row)
        .collect::<Vec<_>>();
    assert_eq!(
        pre_masked, post_masked,
        "only the claim ordinal and the failure budget may change"
    );

    let history = guarded
        .pre
        .fixture_history
        .as_array()
        .expect("fixture history is a JSON array");
    assert_eq!(history.len(), 3, "the history fixture keeps every row");
    let mut attempts = history
        .iter()
        .map(|row| integer_field(row, "attempt"))
        .collect::<Vec<_>>();
    attempts.sort_unstable();
    assert_eq!(attempts, vec![3, 3, 7]);
    let history_ids = history
        .iter()
        .map(|row| integer_field(row, "id"))
        .collect::<HashSet<_>>();
    assert_eq!(
        history_ids.len(),
        3,
        "the duplicated attempt rows keep distinct serial ids"
    );
    assert_eq!(
        guarded.pre.fixture_history, guarded.post.fixture_history,
        "the migration must not rewrite audit history"
    );

    assert_schema_restored(&baseline, &after, migration);
}
