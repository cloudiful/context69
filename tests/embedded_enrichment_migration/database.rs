//! Database checks for the embedded-enrichment retirement.
//!
//! Registered as ignored so the graded suite selects them only through the
//! authorized reusable PostgreSQL runner. The cases keep the durable outbox
//! schema tied to the worker contract and keep the unsafe-cohort refusal tied
//! to its supported-upgrade data-loss boundary.

use context69::db::Database;
use serde_json::json;
use sqlx::{Executor as _, raw_sql};
use uuid::Uuid;

const MIGRATION_SQL: &str =
    include_str!("../../migrations/20261017080707_retire_embedded_enrichment.sql");

async fn connect_scratch() -> Database {
    let url = context69_test_support::accessor::database_url();
    Database::connect(&url)
        .await
        .expect("connect test database")
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn outbox_schema_matches_the_worker_contract() {
    let db = connect_scratch().await;

    let columns =
        sqlx::query_file_scalar!("tests/sql/embedded_enrichment_migration/outbox_columns.sql")
            .fetch_all(db.pool())
            .await
            .expect("read outbox columns");
    for expected in [
        "id",
        "document_id",
        "payload_key",
        "attempts",
        "next_attempt_at",
        "last_error",
        "completed_at",
        "lease_token",
        "lease_expires_at",
        "created_at",
        "updated_at",
    ] {
        assert!(
            columns.iter().any(|column| column == expected),
            "outbox column missing: {expected}"
        );
    }
    for forbidden in ["payload", "result", "metadata_json"] {
        assert!(
            !columns.iter().any(|column| column == forbidden),
            "the outbox must not persist the retired payload: {forbidden}"
        );
    }

    let pending_index = sqlx::query_file_scalar!(
        "tests/sql/embedded_enrichment_migration/pending_index_definition.sql"
    )
    .fetch_one(db.pool())
    .await
    .expect("read pending index");
    assert!(pending_index.contains("completed_at IS NULL"));
    assert!(pending_index.contains("UNIQUE"));
}

/// An affected document without a current-hash canonical snapshot aborts the
/// whole upgrade, so a supported cohort is refused before any data loss.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn unsafe_supported_upgrade_cohort_is_refused_without_data_loss() {
    let db = connect_scratch().await;
    let suffix = Uuid::new_v4().simple().to_string();

    let group_id: i64 = sqlx::query_file_scalar!(
        "tests/sql/embedded_enrichment_migration/insert_group.sql",
        format!("retire-unsafe-{suffix}"),
        format!("Retire Unsafe {suffix}"),
        format!("test/retire-unsafe-{suffix}"),
    )
    .fetch_one(db.pool())
    .await
    .expect("seed group");

    let user_id: i64 = sqlx::query_file_scalar!(
        "tests/sql/embedded_enrichment_migration/insert_user.sql",
        format!("retire-unsafe-{suffix}"),
    )
    .fetch_one(db.pool())
    .await
    .expect("seed user");

    let index_id = Uuid::new_v4();
    sqlx::query_file_scalar!(
        "tests/sql/embedded_enrichment_migration/insert_metadata_index_definition.sql",
        index_id,
        group_id,
        "extraction",
        "ready",
    )
    .fetch_one(db.pool())
    .await
    .expect("seed retired index definition");

    let task_id = Uuid::new_v4();
    sqlx::query_file!(
        "tests/sql/embedded_enrichment_migration/insert_task.sql",
        task_id,
        user_id,
        group_id,
        "failed",
        "extraction",
    )
    .execute(db.pool())
    .await
    .expect("seed labelled task");

    let unsafe_document: i64 = sqlx::query_file_scalar!(
        "tests/sql/embedded_enrichment_migration/insert_unsafe_document.sql",
        group_id,
        format!("retire-unsafe-{suffix}"),
        format!("retire-unsafe-hash-{suffix}"),
        json!({
            "topic": "markets",
            "extraction": {"stock.news.v1": {"direction": "positive"}}
        }),
    )
    .fetch_one(db.pool())
    .await
    .expect("seed unsafe document");

    let mut tx = db
        .pool()
        .begin()
        .await
        .expect("begin migration transaction");
    let error = (&mut *tx)
        .execute(raw_sql(MIGRATION_SQL))
        .await
        .expect_err("an unsafe cohort must refuse the migration");
    assert!(
        error.to_string().contains("canonical snapshot"),
        "unexpected migration error: {error}"
    );
    tx.rollback()
        .await
        .expect("roll back the refused migration");

    let metadata = sqlx::query_file_scalar!(
        "tests/sql/embedded_enrichment_migration/read_document_metadata_only.sql",
        unsafe_document,
    )
    .fetch_one(db.pool())
    .await
    .expect("read unsafe document after rollback");
    assert!(
        metadata.get("extraction").is_some(),
        "the refused upgrade must not strip the source metadata"
    );

    let definitions = sqlx::query_file_scalar!(
        "tests/sql/embedded_enrichment_migration/count_metadata_index_definitions.sql",
        &[index_id],
    )
    .fetch_one(db.pool())
    .await
    .expect("count index definitions after rollback");
    assert_eq!(definitions, 1, "index cleanup must roll back");

    let stage = sqlx::query_file_scalar!(
        "tests/sql/embedded_enrichment_migration/read_task_failure_stage.sql",
        task_id,
    )
    .fetch_one(db.pool())
    .await
    .expect("read task label after rollback");
    assert_eq!(
        stage.as_deref(),
        Some("extraction"),
        "the obsolete label must survive the refused upgrade"
    );

    cleanup_unsafe_case(&db, group_id, user_id, task_id, index_id, unsafe_document).await;
}

async fn cleanup_unsafe_case(
    db: &Database,
    group_id: i64,
    user_id: i64,
    task_id: Uuid,
    index_id: Uuid,
    document_id: i64,
) {
    sqlx::query_file!(
        "tests/sql/embedded_enrichment_migration/delete_task.sql",
        task_id
    )
    .execute(db.pool())
    .await
    .ok();
    sqlx::query_file!(
        "tests/sql/embedded_enrichment_migration/delete_metadata_index_definition.sql",
        index_id
    )
    .execute(db.pool())
    .await
    .ok();
    sqlx::query_file!(
        "tests/sql/embedded_enrichment_migration/delete_document.sql",
        document_id
    )
    .execute(db.pool())
    .await
    .ok();
    sqlx::query_file!(
        "tests/sql/embedded_enrichment_migration/delete_group.sql",
        group_id
    )
    .execute(db.pool())
    .await
    .ok();
    sqlx::query_file!(
        "tests/sql/embedded_enrichment_migration/delete_user.sql",
        user_id
    )
    .execute(db.pool())
    .await
    .ok();
}
