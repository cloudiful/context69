//! Truthful per-item pipeline progress (issue 52 P1).
//!
//! Two halves of one contract:
//!
//! - the bounded `pipeline_progress` payload record: it is versioned, size
//!   capped, validated on read, preserves every other payload key, and rejects
//!   counters it cannot believe instead of rendering them partially;
//! - the `items.sql` projection: it exposes only the progress sub-object, and
//!   only when it belongs to the item's *current* attempt, so a retry or a
//!   resume cannot display a fraction from work an earlier attempt did.
//!
//! The projection half needs a database: it runs only when
//! `CONTEXT69_TEST_DATABASE_URL` points to a scratch database (migrations are
//! applied automatically) and is skipped otherwise.

use context69::services::library::{
    PIPELINE_PROGRESS_KEY, PipelineProgress, PipelineProgressStage, parse_pipeline_progress,
    payload_with_progress, pipeline_progress_from_value,
};
use serde_json::json;
use uuid::Uuid;

static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

// ---------- Record semantics (no database) ----------

fn indexing(
    attempt: i32,
    completed: u32,
    total: u32,
    batch: u32,
    batches: u32,
) -> PipelineProgress {
    PipelineProgress::new(attempt, PipelineProgressStage::Indexing)
        .with_batch_counters(
            completed as usize,
            total as usize,
            batch as usize,
            batches as usize,
        )
        .expect("bounded counters")
}

#[test]
fn a_record_round_trips_and_preserves_every_other_payload_key() {
    let payload = json!({
        "section_payload": [{"section_key": "document"}],
        "url": "https://example.com/a.pdf",
        "indexing_checkpoint": {"v": 1, "next_batch_index": 3, "total_batches": 16},
    });
    let next =
        payload_with_progress(&payload, &indexing(1, 96, 512, 3, 16)).expect("compose payload");
    for key in ["section_payload", "url", "indexing_checkpoint"] {
        assert_eq!(next.get(key), payload.get(key), "`{key}` must survive");
    }
    assert_eq!(
        parse_pipeline_progress(&next),
        Some(indexing(1, 96, 512, 3, 16))
    );
}

#[test]
fn the_record_is_versioned_size_capped_and_content_free() {
    let value = context69::services::library::payload_with_progress(
        &json!({}),
        &indexing(4, 96, 512, 3, 16),
    )
    .expect("compose payload");
    let stored = value.get(PIPELINE_PROGRESS_KEY).expect("stored record");
    let encoded = serde_json::to_string(stored).expect("serializable");
    assert!(
        encoded.len() < 1024,
        "a progress record must stay under its 1024-byte bound, got {}",
        encoded.len()
    );
    for forbidden in ["section_payload", "body_text", "content"] {
        assert!(
            !encoded.contains(forbidden),
            "a record must carry no {forbidden}"
        );
    }
    assert_eq!(stored.get("v").and_then(serde_json::Value::as_u64), Some(1));

    // A foreign version is not progress: it must read as absent rather than
    // as whatever the old format happened to mean.
    let foreign = json!({"pipeline_progress": {"v": 2, "stage": "indexing", "attempt": 1}});
    assert_eq!(parse_pipeline_progress(&foreign), None);
}

#[test]
fn untrustworthy_counters_are_absent_rather_than_partially_shown() {
    for record in [
        // completed beyond total
        json!({"v": 1, "stage": "indexing", "attempt": 1, "completed_chunks": 500, "total_chunks": 100}),
        // completed batches beyond total batches
        json!({"v": 1, "stage": "indexing", "attempt": 1, "completed_batches": 9, "total_batches": 4}),
        // a negative counter must not wrap into a large plausible count
        json!({"v": 1, "stage": "indexing", "attempt": 1, "completed_chunks": -8}),
        // an unknown stage is not a stage
        json!({"v": 1, "stage": "teleporting", "attempt": 1}),
        // not a record at all
        json!("progress"),
    ] {
        assert_eq!(
            pipeline_progress_from_value(&record),
            None,
            "untrustworthy record must be absent: {record}"
        );
    }
}

#[test]
fn unknown_completed_counters_stay_absent_rather_than_zero() {
    // A resume publishes its planned totals before it has re-counted the
    // batches it skipped, so `0` would be a false observation.
    let planned = PipelineProgress::new(2, PipelineProgressStage::Chunking)
        .with_known_totals(512, 16)
        .expect("bounded totals");
    let parsed = parse_pipeline_progress(
        &payload_with_progress(&json!({}), &planned).expect("compose payload"),
    )
    .expect("record parses");
    assert_eq!(parsed.total_chunks, Some(512));
    assert_eq!(parsed.total_batches, Some(16));
    assert_eq!(parsed.completed_chunks, None);
    assert_eq!(parsed.completed_batches, None);
}

#[test]
fn a_document_with_no_chunks_reports_a_real_zero_total() {
    let planned = PipelineProgress::new(1, PipelineProgressStage::Chunking)
        .with_known_totals(0, 0)
        .expect("bounded totals");
    let parsed = parse_pipeline_progress(
        &payload_with_progress(&json!({}), &planned).expect("compose payload"),
    )
    .expect("record parses");
    assert_eq!(parsed.total_chunks, Some(0));
    assert_eq!(parsed.total_batches, Some(0));
}

// ---------- items.sql projection (scratch database) ----------

fn test_database_url() -> Option<String> {
    std::env::var("CONTEXT69_TEST_DATABASE_URL").ok()
}

#[tokio::test(flavor = "multi_thread")]
async fn items_expose_only_the_progress_of_the_current_attempt() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL missing; skipping");
        return;
    };
    let _guard = TEST_LOCK.lock().await;
    let db = context69::db::Database::connect(&url)
        .await
        .expect("connect scratch database");

    let user_id = seed_user(&db).await;
    let task_id = seed_task(&db, user_id).await;
    let item_id = first_item_id(&db, task_id).await;

    // Attempt 1 publishes progress, then the item is retried, which bumps
    // `attempt_count` without touching the payload.
    set_running(&db, item_id, 1).await;
    write_progress(&db, item_id, 1, 96, 512).await;
    assert_eq!(
        projected(&db, task_id, item_id)
            .await
            .as_ref()
            .and_then(|v| v["completed_chunks"].as_i64()),
        Some(96),
        "the owning attempt's own progress is exposed"
    );

    sqlx::query("UPDATE context69.task_items SET attempt_count = 2 WHERE id = $1")
        .bind(item_id)
        .execute(db.pool())
        .await
        .expect("bump attempt");
    assert!(
        projected(&db, task_id, item_id).await.is_none(),
        "progress recorded by an older attempt must not be projected after a retry"
    );

    // The retry publishes its own progress, which is then exposed.
    write_progress(&db, item_id, 2, 10, 512).await;
    let progress = projected(&db, task_id, item_id)
        .await
        .expect("second attempt progress");
    assert_eq!(progress["attempt"].as_i64(), Some(2));
    assert_eq!(progress["completed_chunks"].as_i64(), Some(10));

    // The projection is the bounded sub-object, never the whole payload.
    let payload: serde_json::Value =
        sqlx::query_scalar("SELECT payload FROM context69.task_items WHERE id = $1")
            .bind(item_id)
            .fetch_one(db.pool())
            .await
            .expect("stored payload");
    assert!(
        payload.get("section_payload").is_some(),
        "the fixture payload keeps a large key the projection must not leak"
    );
    assert!(progress.get("section_payload").is_none());
    assert!(progress.get("url").is_none());

    // A record the codec cannot validate is absent rather than partially shown.
    sqlx::query(
        "UPDATE context69.task_items SET payload = jsonb_set(payload, '{pipeline_progress}', $2) WHERE id = $1",
    )
    .bind(item_id)
    .bind(json!({"v": 1, "stage": "indexing", "attempt": 2, "completed_chunks": 5000, "total_chunks": 1}))
    .execute(db.pool())
    .await
    .expect("corrupt record");
    assert!(
        projected(&db, task_id, item_id).await.is_some(),
        "SQL projects the stored sub-object; the service layer rejects it"
    );

    cleanup(&db, task_id, user_id).await;
}

/// A terminal item keeps the final counts of the attempt that produced them,
/// because nothing bumps its attempt afterwards.
#[tokio::test(flavor = "multi_thread")]
async fn a_terminal_item_retains_the_counts_of_its_final_attempt() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL missing; skipping");
        return;
    };
    let _guard = TEST_LOCK.lock().await;
    let db = context69::db::Database::connect(&url)
        .await
        .expect("connect scratch database");

    let user_id = seed_user(&db).await;
    let task_id = seed_task(&db, user_id).await;
    let item_id = first_item_id(&db, task_id).await;

    set_running(&db, item_id, 1).await;
    write_progress(&db, item_id, 1, 512, 512).await;
    sqlx::query(
        "UPDATE context69.task_items SET status = 'succeeded', lease_token = NULL, finished_at = now() WHERE id = $1",
    )
    .bind(item_id)
    .execute(db.pool())
    .await
    .expect("finish item");

    let progress = projected(&db, task_id, item_id)
        .await
        .expect("terminal counts");
    assert_eq!(progress["completed_chunks"].as_i64(), Some(512));
    assert_eq!(progress["total_chunks"].as_i64(), Some(512));
    assert_eq!(progress["stage"].as_str(), Some("indexing"));

    cleanup(&db, task_id, user_id).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn reviewer_finalizing_preserves_persisted_counts_and_checkpoint() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL missing; skipping");
        return;
    };
    let _guard = TEST_LOCK.lock().await;
    let db = context69::db::Database::connect(&url)
        .await
        .expect("connect scratch");
    let user_id = seed_user(&db).await;
    let task_id = seed_task(&db, user_id).await;
    let item_id = first_item_id(&db, task_id).await;
    set_running(&db, item_id, 1).await;
    let lease: Uuid =
        sqlx::query_scalar("SELECT lease_token FROM context69.task_items WHERE id = $1")
            .bind(item_id)
            .fetch_one(db.pool())
            .await
            .expect("lease");
    let indexed_payload = payload_with_progress(
        &json!({"section_payload": [], "indexing_checkpoint": {"v": 1, "next_batch_index": 2, "total_batches": 2}}),
        &indexing(1, 512, 512, 2, 2),
    ).expect("indexed payload");
    assert!(
        db.set_task_item_payload(item_id, lease, &indexed_payload)
            .await
            .expect("indexing write")
    );
    // The indexing service returns its latest payload so finalization retains
    // the checkpoint and counters from this attempt.
    let indexed_progress = parse_pipeline_progress(&indexed_payload).expect("indexing progress");
    let finalizing_progress = PipelineProgress {
        stage: PipelineProgressStage::Finalizing,
        ..indexed_progress
    };
    let final_payload =
        payload_with_progress(&indexed_payload, &finalizing_progress).expect("final payload");
    assert!(
        db.set_task_item_payload(item_id, lease, &final_payload)
            .await
            .expect("finalizing write")
    );
    let stored: serde_json::Value =
        sqlx::query_scalar("SELECT payload FROM context69.task_items WHERE id = $1")
            .bind(item_id)
            .fetch_one(db.pool())
            .await
            .expect("stored payload");
    cleanup(&db, task_id, user_id).await;
    assert_eq!(
        (
            &stored["pipeline_progress"]["completed_chunks"],
            &stored["indexing_checkpoint"]["next_batch_index"],
        ),
        (&json!(512), &json!(2)),
        "finalizing must retain exact completed counts and durable checkpoint",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn reviewer_waiting_retry_hides_previous_attempt_progress() {
    let Some(url) = test_database_url() else {
        eprintln!("CONTEXT69_TEST_DATABASE_URL missing; skipping");
        return;
    };
    let _guard = TEST_LOCK.lock().await;
    let db = context69::db::Database::connect(&url)
        .await
        .expect("connect scratch");
    let user_id = seed_user(&db).await;
    let task_id = seed_task(&db, user_id).await;
    let item_id = first_item_id(&db, task_id).await;
    set_running(&db, item_id, 1).await;
    write_progress(&db, item_id, 1, 96, 512).await;
    for status in ["waiting", "queued"] {
        sqlx::query(
            "UPDATE context69.task_items SET status = $2, \
             waiting_reason = CASE WHEN $2 = 'waiting' THEN 'retry_backoff' ELSE NULL END, \
             lease_token = NULL WHERE id = $1",
        )
        .bind(item_id)
        .bind(status)
        .execute(db.pool())
        .await
        .expect("park or requeue item");
        assert!(
            projected(&db, task_id, item_id).await.is_none(),
            "{status} retry state must suppress old progress before the next claim"
        );
    }
    cleanup(&db, task_id, user_id).await;
}

// ---------- fixture helpers ----------

async fn seed_user(db: &context69::db::Database) -> i64 {
    use sqlx::Row;
    sqlx::query(
        "INSERT INTO context69.users (login_name, display_name, password_hash) \
         VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(format!("pipeline-progress-{}", Uuid::new_v4()))
    .bind("Pipeline Progress Test")
    .bind("unused")
    .fetch_one(db.pool())
    .await
    .expect("seed user")
    .get("id")
}

async fn seed_task(db: &context69::db::Database, user_id: i64) -> Uuid {
    use context69::db::CreateTaskSubmissionRequest;
    let task_id = Uuid::new_v4();
    let (_, _, _) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id,
            user_id,
            group_id: None,
            kind: "text_batch",
            group_path: Some("test/pipeline-progress"),
            source_key: None,
            payloads: &[
                json!({"external_id": "a", "section_payload": [{"section_key": "document"}]}),
            ],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: &format!("pipeline-progress-{}", Uuid::new_v4()),
        })
        .await
        .expect("create task");
    task_id
}

async fn first_item_id(db: &context69::db::Database, task_id: Uuid) -> Uuid {
    sqlx::query_scalar("SELECT id FROM context69.task_items WHERE task_id = $1 ORDER BY ordinal")
        .bind(task_id)
        .fetch_one(db.pool())
        .await
        .expect("task item")
}

async fn set_running(db: &context69::db::Database, item_id: Uuid, attempt: i32) {
    sqlx::query(
        "UPDATE context69.task_items SET status = 'running', attempt_count = $2, \
         lease_token = $3, lease_until = now() + interval '5 minutes' WHERE id = $1",
    )
    .bind(item_id)
    .bind(attempt)
    .bind(Uuid::new_v4())
    .execute(db.pool())
    .await
    .expect("claim-like setup");
}

async fn write_progress(
    db: &context69::db::Database,
    item_id: Uuid,
    attempt: i32,
    completed: u32,
    total: u32,
) {
    let progress = indexing(attempt, completed, total, 1, 2);
    sqlx::query("UPDATE context69.task_items SET payload = $2 WHERE id = $1")
        .bind(item_id)
        .bind(
            payload_with_progress(
                &json!({"external_id": "a", "section_payload": [{"section_key": "document"}]}),
                &progress,
            )
            .expect("compose payload"),
        )
        .execute(db.pool())
        .await
        .expect("write progress");
}

async fn projected(
    db: &context69::db::Database,
    task_id: Uuid,
    item_id: Uuid,
) -> Option<serde_json::Value> {
    let rows = db
        .list_task_items(task_id, 10, 0)
        .await
        .expect("list task items");
    let item = rows
        .iter()
        .find(|row| row.id == item_id)
        .expect("the item is projected")
        .clone();
    assert!(
        rows.iter()
            .all(|row| row.id == item_id || row.pipeline_progress.is_none()),
        "a task without progress must not fabricate it for another item"
    );
    item.pipeline_progress
}

async fn cleanup(db: &context69::db::Database, task_id: Uuid, user_id: i64) {
    sqlx::query("DELETE FROM context69.task_items WHERE task_id = $1")
        .bind(task_id)
        .execute(db.pool())
        .await
        .expect("cleanup items");
    sqlx::query("DELETE FROM context69.tasks WHERE id = $1")
        .bind(task_id)
        .execute(db.pool())
        .await
        .expect("cleanup task");
    sqlx::query("DELETE FROM context69.task_idempotency_keys WHERE user_id = $1")
        .bind(user_id)
        .execute(db.pool())
        .await
        .expect("cleanup idempotency");
    sqlx::query("DELETE FROM context69.users WHERE id = $1")
        .bind(user_id)
        .execute(db.pool())
        .await
        .expect("cleanup user");
}
