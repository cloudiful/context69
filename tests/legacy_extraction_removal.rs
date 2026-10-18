//! Issue #910 P2 legacy extraction removal tests.
//!
//! These run only through `./scripts/test-backend.sh`, which resolves the
//! reusable test PostgreSQL target and supplies it as
//! `CONTEXT69_TEST_DATABASE_URL`; they are skipped (reported `ignored`) by a
//! plain `cargo test --workspace` and are reconciled against
//! `crates/context69-test-support/tests/database-tests.json`.
//!
//! They assert the two halves of the #910 contract: the four dedicated legacy
//! extraction tables and the three `library_files` directive columns are gone,
//! while unrelated business data (source documents and their hashed snapshots,
//! chunks, task history, translation provider rows and the P1 generic results)
//! survives unchanged. The retirement migration runs automatically when
//! `Database::connect` opens the deployment database; rows created after it are
//! never revisited, so this fixture also proves that a later source generation
//! is not rewritten by the schema retirement.

use context69::db::Database;
use serde_json::json;
use sqlx::Row as _;
use uuid::Uuid;

async fn connect_scratch() -> Database {
    let url = context69_test_support::accessor::database_url();
    Database::connect(&url)
        .await
        .expect("connect test database")
}

async fn seed_group(db: &Database) -> i64 {
    let suffix = Uuid::new_v4().simple().to_string();
    sqlx::query(
        "INSERT INTO context69.groups (group_key, name, visibility, kind, full_path) \
         VALUES ($1, $2, 'public', 'shared', $3) RETURNING id",
    )
    .bind(format!("legacy-removal-{suffix}"))
    .bind(format!("Legacy Removal {suffix}"))
    .bind(format!("test/legacy-removal-{suffix}"))
    .fetch_one(db.pool())
    .await
    .expect("seed group")
    .get("id")
}

async fn seed_user(db: &Database) -> i64 {
    sqlx::query(
        "INSERT INTO context69.users (login_name, display_name, password_hash) \
         VALUES ($1, 'Legacy Removal', 'unused') RETURNING id",
    )
    .bind(format!("legacy-removal-{}", Uuid::new_v4()))
    .fetch_one(db.pool())
    .await
    .expect("seed user")
    .get("id")
}

/// A document whose source metadata still carries the retired `extraction`
/// namespace. The automatic retirement migration processes existing rows once
/// at deployment and never rewrites a source generation it has already passed,
/// so this later fixture proves the canonical snapshot and chunks stay intact.
async fn seed_document_with_extraction_metadata(db: &Database, group_id: i64) -> (i64, String) {
    let record_hash = format!("legacy-removal-{}", Uuid::new_v4());
    let metadata = json!({
        "ts_code": "600519.SH",
        "extraction": {"stock.news.v1": {"direction": "positive"}}
    });
    let id = sqlx::query(
        "INSERT INTO context69.documents \
         (group_id, source_key, external_id, title, summary, source_uri, \
          updated_at_source, record_hash, metadata_json, visibility) \
         VALUES ($1, 'legacy-removal-src', $2, 'Legacy Doc', NULL, \
                 'https://example.test/legacy-removal', now(), $3, $4, 'public') \
         RETURNING id",
    )
    .bind(group_id)
    .bind(format!("legacy-removal-{}", Uuid::new_v4()))
    .bind(&record_hash)
    .bind(&metadata)
    .fetch_one(db.pool())
    .await
    .expect("seed document")
    .get("id");
    (id, record_hash)
}

async fn cleanup(db: &Database, group_id: i64, document_id: i64, user_id: i64) {
    // Results and versions/chunks cascade from the document; task rows cascade
    // from the group. Order matters only for the RESTRICT author FK: results
    // must go before the user.
    for (statement, value) in [
        (
            "DELETE FROM context69.document_results WHERE document_id = $1",
            document_id,
        ),
        (
            "DELETE FROM context69.document_versions WHERE document_id = $1",
            document_id,
        ),
        (
            "DELETE FROM context69.document_chunks WHERE document_id = $1",
            document_id,
        ),
        ("DELETE FROM context69.documents WHERE id = $1", document_id),
        ("DELETE FROM context69.tasks WHERE group_id = $1", group_id),
        ("DELETE FROM context69.groups WHERE id = $1", group_id),
        ("DELETE FROM context69.users WHERE id = $1", user_id),
    ] {
        sqlx::query(statement)
            .bind(value)
            .execute(db.pool())
            .await
            .ok();
    }
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn legacy_extraction_tables_and_columns_are_gone() {
    let db = connect_scratch().await;

    for table in [
        "document_extraction_attempts",
        "document_extraction_versions",
        "document_extraction_jobs",
        "extraction_templates",
    ] {
        let present: bool = sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
            .bind(format!("context69.{table}"))
            .fetch_one(db.pool())
            .await
            .expect("read regclass");
        assert!(!present, "legacy extraction table must be dropped: {table}");
    }

    let columns: Vec<String> = sqlx::query(
        "SELECT column_name FROM information_schema.columns \
         WHERE table_schema = 'context69' AND table_name = 'library_files'",
    )
    .fetch_all(db.pool())
    .await
    .expect("read library_files columns")
    .into_iter()
    .map(|row| row.get::<String, _>("column_name"))
    .collect();
    for dropped in [
        "extraction_template_key",
        "extraction_parameters",
        "extraction_required",
    ] {
        assert!(
            !columns.iter().any(|column| column == dropped),
            "dropped library_files column must be absent: {dropped}"
        );
    }
    // Unrelated directive/business columns survive the drop.
    assert!(
        columns
            .iter()
            .any(|column| column == "translation_override")
    );
    assert!(columns.iter().any(|column| column == "sha256"));
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn source_generations_tasks_translation_and_results_survive_legacy_table_removal() {
    let db = connect_scratch().await;
    let group_id = seed_group(&db).await;
    let user_id = seed_user(&db).await;
    let (document_id, record_hash) = seed_document_with_extraction_metadata(&db, group_id).await;

    // Immutable source snapshot: the canonical version keeps the business
    // metadata that the retired embedded key was merged over, plus a chunk at
    // the same generation. The schema retirement must not rewrite or delete
    // these.
    sqlx::query(
        "INSERT INTO context69.document_versions \
         (document_id, record_hash, title, summary, body_text, source_uri, metadata_json) \
         VALUES ($1, $2, 'Legacy Doc', NULL, 'alpha\nbeta', \
                 'https://example.test/legacy-removal', \
                 '{\"ts_code\": \"600519.SH\"}'::jsonb)",
    )
    .bind(document_id)
    .bind(&record_hash)
    .execute(db.pool())
    .await
    .expect("seed version snapshot");
    sqlx::query(
        "INSERT INTO context69.document_chunks \
         (id, document_id, chunk_index, chunk_text, record_hash) \
         VALUES ($1, $2, 0, 'alpha', $3), ($4, $2, 1, 'beta', $3)",
    )
    .bind(Uuid::new_v4())
    .bind(document_id)
    .bind(&record_hash)
    .bind(Uuid::new_v4())
    .execute(db.pool())
    .await
    .expect("seed chunks");

    // Task history: parent + item + attempt survive the cleanup.
    let task_id = Uuid::new_v4();
    let item_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO context69.tasks \
         (id, user_id, group_id, kind, status, group_path, total_count) \
         VALUES ($1, $2, $3, 'file_batch', 'succeeded', 'test/legacy-removal', 1)",
    )
    .bind(task_id)
    .bind(user_id)
    .bind(group_id)
    .execute(db.pool())
    .await
    .expect("seed task");
    sqlx::query(
        "INSERT INTO context69.task_items (id, task_id, ordinal, payload, status) \
         VALUES ($1, $2, 0, '{}'::jsonb, 'succeeded')",
    )
    .bind(item_id)
    .bind(task_id)
    .execute(db.pool())
    .await
    .expect("seed task item");
    sqlx::query(
        "INSERT INTO context69.task_attempts (task_id, item_id, attempt, status) \
         VALUES ($1, $2, 1, 'succeeded')",
    )
    .bind(task_id)
    .bind(item_id)
    .execute(db.pool())
    .await
    .expect("seed task attempt");

    // P1 generic result appended by an external caller.
    sqlx::query(
        "INSERT INTO context69.document_results \
         (document_id, author_user_id, producer, result_kind, schema_ref, \
          source_record_hash, idempotency_key, result, provenance) \
         VALUES ($1, $2, 'stock.news', 'sentiment.v1', 'stock.news/sentiment@1', \
                 $3, 'legacy-removal-run', '{\"direction\": \"positive\"}'::jsonb, NULL)",
    )
    .bind(document_id)
    .bind(user_id)
    .bind(&record_hash)
    .execute(db.pool())
    .await
    .expect("seed document result");

    // Assertions: every preserved row is intact.
    let (metadata, hash): (serde_json::Value, String) =
        sqlx::query_as("SELECT metadata_json, record_hash FROM context69.documents WHERE id = $1")
            .bind(document_id)
            .fetch_one(db.pool())
            .await
            .expect("document survives");
    assert_eq!(hash, record_hash, "record_hash must be unchanged");
    assert_eq!(
        metadata["ts_code"], "600519.SH",
        "non-enrichment source metadata must be preserved"
    );

    let (version_metadata, version_body): (serde_json::Value, String) = sqlx::query_as(
        "SELECT metadata_json, body_text FROM context69.document_versions \
         WHERE document_id = $1 AND record_hash = $2",
    )
    .bind(document_id)
    .bind(&record_hash)
    .fetch_one(db.pool())
    .await
    .expect("version snapshot survives");
    assert_eq!(version_metadata["ts_code"], "600519.SH");
    assert_eq!(
        version_body, "alpha\nbeta",
        "canonical snapshot body must not change"
    );

    let chunk_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM context69.document_chunks WHERE document_id = $1")
            .bind(document_id)
            .fetch_one(db.pool())
            .await
            .expect("chunks survive");
    assert_eq!(chunk_count, 2, "chunks must be preserved");

    let task_count: i64 = sqlx::query_scalar("SELECT count(*) FROM context69.tasks WHERE id = $1")
        .bind(task_id)
        .fetch_one(db.pool())
        .await
        .expect("task survives");
    assert_eq!(task_count, 1, "task history must be preserved");
    let attempt_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM context69.task_attempts WHERE task_id = $1")
            .bind(task_id)
            .fetch_one(db.pool())
            .await
            .expect("attempt survives");
    assert_eq!(attempt_count, 1, "task attempts must be preserved");

    // The shared translation provider row is untouched by the removal.
    let llm_providers: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM context69.translation_provider_settings WHERE provider_key = 'llm'",
    )
    .fetch_one(db.pool())
    .await
    .expect("translation provider survives");
    assert_eq!(
        llm_providers, 1,
        "translation provider row must be preserved"
    );

    // The generic caller result survives and keeps its bound source hash.
    let (result, bound_hash): (serde_json::Value, String) = sqlx::query_as(
        "SELECT result, source_record_hash FROM context69.document_results WHERE document_id = $1",
    )
    .bind(document_id)
    .fetch_one(db.pool())
    .await
    .expect("generic result survives");
    assert_eq!(result["direction"], "positive");
    assert_eq!(bound_hash, record_hash);

    cleanup(&db, group_id, document_id, user_id).await;
}
