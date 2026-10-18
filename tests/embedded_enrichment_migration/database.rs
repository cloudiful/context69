//! Post-checkpoint database checks for the retirement migration.
//!
//! Registered as ignored so the graded suite selects them only through the
//! authorized reusable PostgreSQL runner. The migration cases re-run the
//! authored SQL inside a transaction that is always rolled back, so they can
//! observe a zero-affected rollout and a fail-closed rollback without leaving
//! any state behind.

use context69::db::Database;
use serde_json::json;
use sqlx::{Acquire as _, Executor as _, PgConnection, raw_sql};
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

async fn seed_index(
    conn: &mut PgConnection,
    group_id: i64,
    field_path: &str,
    status: &str,
) -> Uuid {
    sqlx::query_file_scalar!(
        "tests/sql/embedded_enrichment_migration/insert_metadata_index_definition.sql",
        Uuid::new_v4(),
        group_id,
        field_path,
        status,
    )
    .fetch_one(conn)
    .await
    .expect("seed metadata index definition")
}

struct Fixture {
    group_id: i64,
    user_id: i64,
    document_id: i64,
    extraction_index: Uuid,
    scoped_index: Uuid,
    topic_index: Uuid,
    extraction_task: Uuid,
    unrelated_task: Uuid,
    extraction_item: Uuid,
    unrelated_item: Uuid,
}

async fn seed_fixture(conn: &mut PgConnection) -> Fixture {
    let suffix = Uuid::new_v4().simple().to_string();
    let group_id: i64 = sqlx::query_file_scalar!(
        "tests/sql/embedded_enrichment_migration/insert_group.sql",
        format!("retire-migration-{suffix}"),
        format!("Retire Migration {suffix}"),
        format!("test/retire-migration-{suffix}"),
    )
    .fetch_one(&mut *conn)
    .await
    .expect("seed group");

    let user_id: i64 = sqlx::query_file_scalar!(
        "tests/sql/embedded_enrichment_migration/insert_user.sql",
        format!("retire-migration-{suffix}"),
    )
    .fetch_one(&mut *conn)
    .await
    .expect("seed user");

    let document_id: i64 = sqlx::query_file_scalar!(
        "tests/sql/embedded_enrichment_migration/insert_document.sql",
        group_id,
        format!("retire-migration-{suffix}"),
        format!("retire-migration-hash-{suffix}"),
        json!({"topic": "markets"}),
    )
    .fetch_one(&mut *conn)
    .await
    .expect("seed document");

    let extraction_index = seed_index(conn, group_id, "extraction", "ready").await;
    let scoped_index = seed_index(conn, group_id, "extraction.confidence", "ready").await;
    let topic_index = seed_index(conn, group_id, "topic", "ready").await;

    for index_id in [extraction_index, scoped_index, topic_index] {
        sqlx::query_file!(
            "tests/sql/embedded_enrichment_migration/insert_metadata_value.sql",
            index_id,
            document_id,
        )
        .execute(&mut *conn)
        .await
        .expect("seed metadata value");
    }

    let extraction_task = Uuid::new_v4();
    let unrelated_task = Uuid::new_v4();
    for (task_id, stage, status) in [
        (extraction_task, "extraction", "failed"),
        (unrelated_task, "lease", "failed"),
    ] {
        sqlx::query_file!(
            "tests/sql/embedded_enrichment_migration/insert_task.sql",
            task_id,
            user_id,
            group_id,
            status,
            stage,
        )
        .execute(&mut *conn)
        .await
        .expect("seed task");
    }

    let extraction_item = Uuid::new_v4();
    let unrelated_item = Uuid::new_v4();
    for (item_id, task_id, stage) in [
        (extraction_item, extraction_task, "extraction"),
        (unrelated_item, unrelated_task, "lease"),
    ] {
        sqlx::query_file!(
            "tests/sql/embedded_enrichment_migration/insert_task_item.sql",
            item_id,
            task_id,
            stage,
        )
        .execute(&mut *conn)
        .await
        .expect("seed task item");
    }

    Fixture {
        group_id,
        user_id,
        document_id,
        extraction_index,
        scoped_index,
        topic_index,
        extraction_task,
        unrelated_task,
        extraction_item,
        unrelated_item,
    }
}

async fn cleanup(conn: &mut PgConnection, fixture: &Fixture) {
    for task_id in [fixture.extraction_task, fixture.unrelated_task] {
        sqlx::query_file!(
            "tests/sql/embedded_enrichment_migration/delete_task.sql",
            task_id
        )
        .execute(&mut *conn)
        .await
        .ok();
    }
    for index_id in [
        fixture.extraction_index,
        fixture.scoped_index,
        fixture.topic_index,
    ] {
        sqlx::query_file!(
            "tests/sql/embedded_enrichment_migration/delete_metadata_index_definition.sql",
            index_id
        )
        .execute(&mut *conn)
        .await
        .ok();
    }
    sqlx::query_file!(
        "tests/sql/embedded_enrichment_migration/delete_document.sql",
        fixture.document_id
    )
    .execute(&mut *conn)
    .await
    .ok();
    sqlx::query_file!(
        "tests/sql/embedded_enrichment_migration/delete_group.sql",
        fixture.group_id
    )
    .execute(&mut *conn)
    .await
    .ok();
    sqlx::query_file!(
        "tests/sql/embedded_enrichment_migration/delete_user.sql",
        fixture.user_id
    )
    .execute(&mut *conn)
    .await
    .ok();
}

/// Zero documents still carry the key: the rollout must still clear only the
/// obsolete label and the retired index definitions, without an admin lookup
/// and without touching unrelated labels, counters, errors, or index rows.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn zero_affected_documents_still_clear_obsolete_labels_and_index_definitions() {
    let db = connect_scratch().await;
    let mut conn = db.pool().acquire().await.expect("acquire connection");

    let extraction_documents = sqlx::query_file_scalar!(
        "tests/sql/embedded_enrichment_migration/count_extraction_documents.sql"
    )
    .fetch_one(&mut *conn)
    .await
    .expect("count affected documents");
    assert_eq!(
        extraction_documents, 0,
        "this case needs a database without affected documents"
    );

    let fixture = seed_fixture(&mut conn).await;
    let generation_before = sqlx::query_file_scalar!(
        "tests/sql/embedded_enrichment_migration/read_search_generation.sql"
    )
    .fetch_one(&mut *conn)
    .await
    .expect("read search generation");

    let mut tx = (&mut *conn)
        .begin()
        .await
        .expect("begin migration transaction");
    (&mut *tx)
        .execute(raw_sql(MIGRATION_SQL))
        .await
        .expect("run retirement migration");

    let rows = sqlx::query_file!(
        "tests/sql/embedded_enrichment_migration/read_task_states.sql",
        &[fixture.extraction_task, fixture.unrelated_task],
    )
    .fetch_all(&mut *tx)
    .await
    .expect("read tasks");
    for row in rows {
        assert_eq!(row.status, "failed", "task status must not change");
        assert_eq!(row.failed_count, 1, "task counters must not change");
        assert_eq!(row.error_summary.as_deref(), Some("kept summary"));
        if row.id == fixture.extraction_task {
            assert_eq!(
                row.failure_stage, None,
                "the obsolete task label must be cleared"
            );
        } else {
            assert_eq!(
                row.failure_stage.as_deref(),
                Some("lease"),
                "unrelated labels must be preserved"
            );
        }
    }

    let rows = sqlx::query_file!(
        "tests/sql/embedded_enrichment_migration/read_task_item_states.sql",
        &[fixture.extraction_item, fixture.unrelated_item],
    )
    .fetch_all(&mut *tx)
    .await
    .expect("read task items");
    for row in rows {
        assert_eq!(row.status, "failed");
        assert_eq!(row.error_message.as_deref(), Some("kept error"));
        assert_eq!(row.attempt_count, 2, "attempt counters must not change");
        if row.id == fixture.extraction_item {
            assert_eq!(
                row.failure_stage, None,
                "the obsolete item label must be cleared"
            );
        } else {
            assert_eq!(
                row.failure_stage.as_deref(),
                Some("lease"),
                "unrelated item labels must be preserved"
            );
        }
    }

    let remaining = sqlx::query_file_scalar!(
        "tests/sql/embedded_enrichment_migration/read_index_field_paths.sql",
        fixture.group_id,
    )
    .fetch_all(&mut *tx)
    .await
    .expect("read index definitions");
    assert_eq!(
        remaining,
        vec!["topic".to_string()],
        "only extraction/extraction.* definitions are removed"
    );
    let values = sqlx::query_file_scalar!(
        "tests/sql/embedded_enrichment_migration/count_metadata_values.sql",
        &[
            fixture.extraction_index,
            fixture.scoped_index,
            fixture.topic_index,
        ],
    )
    .fetch_one(&mut *tx)
    .await
    .expect("count metadata values");
    assert_eq!(values, 1, "removed definitions cascade their values");

    let results = sqlx::query_file_scalar!(
        "tests/sql/embedded_enrichment_migration/count_document_results.sql"
    )
    .fetch_one(&mut *tx)
    .await
    .expect("count results");
    assert_eq!(results, 0, "zero affected documents must import nothing");
    let jobs = sqlx::query_file_scalar!(
        "tests/sql/embedded_enrichment_migration/count_payload_sync_jobs.sql"
    )
    .fetch_one(&mut *tx)
    .await
    .expect("count outbox jobs");
    assert_eq!(jobs, 0, "zero affected documents must enqueue nothing");

    let document = sqlx::query_file!(
        "tests/sql/embedded_enrichment_migration/read_document_metadata.sql",
        fixture.document_id,
    )
    .fetch_one(&mut *tx)
    .await
    .expect("read document");
    assert_eq!(document.metadata_json, json!({"topic": "markets"}));
    assert!(document.record_hash.starts_with("retire-migration-hash-"));

    let generation_after = sqlx::query_file_scalar!(
        "tests/sql/embedded_enrichment_migration/read_search_generation.sql"
    )
    .fetch_one(&mut *tx)
    .await
    .expect("read search generation after cleanup");
    assert_eq!(
        generation_after,
        generation_before + 1,
        "removing index definitions must invalidate cached search once"
    );

    tx.rollback()
        .await
        .expect("roll back migration transaction");
    cleanup(&mut conn, &fixture).await;
}

/// An unsafe affected document aborts the whole transaction, so even the
/// otherwise independent label/index cleanup is rolled back.
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn unsafe_document_rolls_back_every_cleanup_change() {
    let db = connect_scratch().await;
    let mut conn = db.pool().acquire().await.expect("acquire connection");
    let fixture = seed_fixture(&mut conn).await;

    // Affected metadata without a current-hash canonical snapshot: the first
    // guard must refuse the cohort.
    let unsafe_document: i64 = sqlx::query_file_scalar!(
        "tests/sql/embedded_enrichment_migration/insert_unsafe_document.sql",
        fixture.group_id,
        format!("retire-unsafe-{}", Uuid::new_v4()),
        format!("retire-unsafe-hash-{}", Uuid::new_v4()),
        json!({
            "topic": "markets",
            "extraction": {"stock.news.v1": {"direction": "positive"}}
        }),
    )
    .fetch_one(&mut *conn)
    .await
    .expect("seed unsafe document");

    let mut tx = (&mut *conn)
        .begin()
        .await
        .expect("begin migration transaction");
    let error = (&mut *tx)
        .execute(raw_sql(MIGRATION_SQL))
        .await
        .expect_err("unsafe cohort must abort the migration");
    assert!(
        error.to_string().contains("canonical snapshot"),
        "unexpected migration error: {error}"
    );
    tx.rollback().await.expect("roll back aborted migration");

    let stage = sqlx::query_file_scalar!(
        "tests/sql/embedded_enrichment_migration/read_task_failure_stage.sql",
        fixture.extraction_task,
    )
    .fetch_one(&mut *conn)
    .await
    .expect("read task stage after rollback");
    assert_eq!(
        stage.as_deref(),
        Some("extraction"),
        "the obsolete label must survive an aborted cohort"
    );
    let definitions = sqlx::query_file_scalar!(
        "tests/sql/embedded_enrichment_migration/count_metadata_index_definitions.sql",
        &[fixture.extraction_index, fixture.scoped_index],
    )
    .fetch_one(&mut *conn)
    .await
    .expect("count definitions after rollback");
    assert_eq!(definitions, 2, "index cleanup must roll back too");
    let metadata = sqlx::query_file_scalar!(
        "tests/sql/embedded_enrichment_migration/read_document_metadata_only.sql",
        unsafe_document,
    )
    .fetch_one(&mut *conn)
    .await
    .expect("read unsafe document after rollback");
    assert!(metadata.get("extraction").is_some());

    sqlx::query_file!(
        "tests/sql/embedded_enrichment_migration/delete_document.sql",
        unsafe_document
    )
    .execute(&mut *conn)
    .await
    .ok();
    cleanup(&mut conn, &fixture).await;
}
