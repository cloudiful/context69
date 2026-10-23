//! Issue #1041 P1 database checks for the guarded library-file dedup index.
//!
//! These run only through `./scripts/test-backend.sh`, which resolves the
//! reusable test PostgreSQL target and supplies it as
//! `CONTEXT69_TEST_DATABASE_URL`; they are reported `ignored` by a plain
//! `cargo test --workspace` and are reconciled against
//! `crates/context69-test-support/tests/database-tests.json`.
//!
//! The runner gives every target a freshly reset `context69` schema with the
//! real migrator applied once, so the index under test is the one the shipped
//! migration creates. The cases that exercise the migration's compatibility
//! guard re-run the compiled migration inside a transaction that is always
//! rolled back, so no fixture state survives the test and no production
//! database is involved.

use context69::db::{Database, MIGRATOR};
use sqlx::{PgConnection, raw_sql};

const MIGRATION_DESCRIPTION: &str = "library files group sha256 index";
const INDEX_NAME: &str = "idx_library_files_group_sha256";

const READ_INDEX_SHAPE: &str = include_str!("library_files_sha256_index/read_index_shape.sql");
const READ_INDEX_PRESENCE: &str =
    include_str!("library_files_sha256_index/read_index_presence.sql");
const DROP_DEDUP_INDEX: &str = include_str!("library_files_sha256_index/drop_dedup_index.sql");
const CREATE_EQUIVALENT_INDEX: &str =
    include_str!("library_files_sha256_index/create_equivalent_index.sql");
const CREATE_INCOMPATIBLE_INDEX: &str =
    include_str!("library_files_sha256_index/create_incompatible_index.sql");
const CREATE_SAME_NAME_TABLE: &str =
    include_str!("library_files_sha256_index/create_same_name_table.sql");

/// The catalog facts the migration's compatibility guard decides on.
#[derive(Debug, sqlx::FromRow)]
struct IndexShape {
    relation_kind: String,
    access_method: String,
    indexed_schema: String,
    indexed_table: String,
    is_unique: bool,
    is_valid: bool,
    is_ready: bool,
    is_live: bool,
    has_predicate: bool,
    has_expressions: bool,
    key_column_count: i32,
    total_column_count: i32,
    key_columns: Vec<String>,
}

async fn connect_scratch() -> Database {
    let url = context69_test_support::accessor::database_url();
    Database::connect(&url)
        .await
        .expect("connect test database")
}

/// The SQL of the migration this build actually compiles.
fn migration_sql() -> &'static str {
    let migrator: &'static sqlx::migrate::Migrator = &MIGRATOR;
    let migration = migrator
        .iter()
        .find(|migration| migration.description == MIGRATION_DESCRIPTION)
        .expect("the compiled migrator embeds the dedup index migration");
    migration.sql.as_str()
}

async fn read_index_shape(connection: &mut PgConnection) -> Option<IndexShape> {
    sqlx::query_as::<_, IndexShape>(READ_INDEX_SHAPE)
        .fetch_optional(connection)
        .await
        .expect("read the dedup index shape")
}

async fn index_present(connection: &mut PgConnection) -> bool {
    sqlx::query_scalar::<_, bool>(READ_INDEX_PRESENCE)
        .fetch_one(connection)
        .await
        .expect("read the dedup index presence")
}

/// The shape the lookup needs: a valid, ready, live, non-unique btree over
/// exactly the dedup equality pair followed by the lookup order.
fn assert_expected_lookup_index(shape: &IndexShape) {
    assert_eq!(
        shape.relation_kind, "i",
        "the dedup lookup must be an index"
    );
    assert_eq!(
        shape.access_method, "btree",
        "the dedup lookup index must use btree"
    );
    assert_eq!(shape.indexed_schema, "context69");
    assert_eq!(shape.indexed_table, "library_files");
    assert!(
        !shape.is_unique,
        "the dedup lookup index must not add a uniqueness rule"
    );
    assert!(
        shape.is_valid && shape.is_ready && shape.is_live,
        "the dedup lookup index must be valid, ready, and live"
    );
    assert!(
        !shape.has_predicate,
        "the dedup lookup index must not be partial"
    );
    assert!(
        !shape.has_expressions,
        "the dedup lookup index must not use expression columns"
    );
    assert_eq!(shape.key_column_count, 4);
    assert_eq!(
        shape.total_column_count, 4,
        "the dedup lookup index must not carry INCLUDE columns"
    );
    assert_eq!(
        shape.key_columns,
        vec!["group_id", "sha256", "created_at", "id"],
        "the dedup lookup index must cover the equality pair and its order"
    );
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn the_dedup_lookup_index_is_applied_with_the_expected_shape() {
    let db = connect_scratch().await;
    let mut connection = db.pool().acquire().await.expect("acquire a connection");
    let shape = read_index_shape(&mut connection)
        .await
        .expect("the migration must have applied the dedup lookup index");
    assert_expected_lookup_index(&shape);
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn the_migration_creates_the_dedup_lookup_index_when_no_same_name_object_exists() {
    let db = connect_scratch().await;
    let mut transaction = db
        .pool()
        .begin()
        .await
        .expect("begin a fixture transaction");

    sqlx::query(DROP_DEDUP_INDEX)
        .execute(&mut *transaction)
        .await
        .expect("remove the applied dedup index");
    assert!(
        !index_present(&mut transaction).await,
        "the fixture must start without a same-name object"
    );

    raw_sql(migration_sql())
        .execute(&mut *transaction)
        .await
        .expect("the migration must create the dedup lookup index");

    let shape = read_index_shape(&mut transaction)
        .await
        .expect("the migration must have created the dedup lookup index");
    assert_expected_lookup_index(&shape);

    transaction.rollback().await.ok();
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn the_migration_accepts_an_equivalent_precreated_dedup_index() {
    let db = connect_scratch().await;
    let mut transaction = db
        .pool()
        .begin()
        .await
        .expect("begin a fixture transaction");

    sqlx::query(DROP_DEDUP_INDEX)
        .execute(&mut *transaction)
        .await
        .expect("remove the applied dedup index");
    sqlx::query(CREATE_EQUIVALENT_INDEX)
        .execute(&mut *transaction)
        .await
        .expect("pre-create the equivalent dedup index");

    raw_sql(migration_sql())
        .execute(&mut *transaction)
        .await
        .expect("the migration must accept an equivalent pre-created index");

    let shape = read_index_shape(&mut transaction)
        .await
        .expect("the pre-created index must still be in place");
    assert_expected_lookup_index(&shape);

    transaction.rollback().await.ok();
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn the_migration_rejects_an_incompatible_same_name_index() {
    let db = connect_scratch().await;
    let mut transaction = db
        .pool()
        .begin()
        .await
        .expect("begin a fixture transaction");

    sqlx::query(DROP_DEDUP_INDEX)
        .execute(&mut *transaction)
        .await
        .expect("remove the applied dedup index");
    sqlx::query(CREATE_INCOMPATIBLE_INDEX)
        .execute(&mut *transaction)
        .await
        .expect("create the incompatible same-name index");

    let error = match raw_sql(migration_sql()).execute(&mut *transaction).await {
        Ok(_) => panic!("the migration must reject an incompatible same-name index"),
        Err(error) => error,
    };
    let message = error.to_string();
    assert!(
        message.contains("library file dedup index guard failed"),
        "the refusal must name the guard that refused the object: {message}"
    );
    assert!(
        message.contains(INDEX_NAME),
        "the refusal must name the object it refused: {message}"
    );

    transaction.rollback().await.ok();
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn the_migration_rejects_a_same_name_object_that_is_not_an_index() {
    let db = connect_scratch().await;
    let mut transaction = db
        .pool()
        .begin()
        .await
        .expect("begin a fixture transaction");

    sqlx::query(DROP_DEDUP_INDEX)
        .execute(&mut *transaction)
        .await
        .expect("remove the applied dedup index");
    sqlx::query(CREATE_SAME_NAME_TABLE)
        .execute(&mut *transaction)
        .await
        .expect("create the same-name non-index object");

    let error = match raw_sql(migration_sql()).execute(&mut *transaction).await {
        Ok(_) => panic!("the migration must reject a same-name object that is not an index"),
        Err(error) => error,
    };
    let message = error.to_string();
    assert!(
        message.contains("library file dedup index guard failed"),
        "the refusal must name the guard that refused the object: {message}"
    );
    assert!(
        message.contains("not an index"),
        "the refusal must say that the object is not an index: {message}"
    );

    transaction.rollback().await.ok();
}
