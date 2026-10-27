//! The automatic startup handover of the legacy task items (issue 1100 P1).
//!
//! The task center executes its items through the `cloudiful_tasks` library. An
//! item that was created before the library owned the execution carries no
//! segment, so the engine refuses to start while such an item exists and the
//! operator hands it over with the explicit tool.
//!
//! These cases pin the automatic form of that handover on the real rows: a
//! startup adopts the queued and waiting legacy items with one segment each,
//! closes an expired running item as interrupted so the user can retry it, and
//! it stops at its bound, refuses a live lease and reports the rows it cannot
//! resolve instead of claiming that the database is ready.
//!
//! The last case boots the real application, so it belongs to the opt-in
//! application startup window like every other application boot.

use context69::config::Config;
use context69::db::{ClaimedItem, Database};
use context69::services::app::Context69App;
use context69::services::tasks::handover::{
    HandoverBlocker, HandoverLimits, MAX_BATCH_SIZE, MAX_BATCHES, run_legacy_handover,
};
use uuid::Uuid;

static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Unreachable loopback endpoints: the connection is refused immediately, so
/// the booted application can never reach a real Qdrant or embedding server.
const SENTINEL_URL: &str = "http://127.0.0.1:1";
const SENTINEL_EMBEDDING_URL: &str = "http://127.0.0.1:1/v1";

async fn connect() -> Database {
    let url = context69_test_support::accessor::database_url();
    let db = Database::connect(&url)
        .await
        .expect("connect test database");
    context69_db_schema::bootstrap_library_schema(db.pool())
        .await
        .expect("bootstrap the library schema");
    db
}

/// The production bounds of one handover run.
fn production_limits() -> HandoverLimits {
    HandoverLimits::default()
}

/// The legacy population the database carries before the case runs.
///
/// The scratch database is shared, so every case states its expectation
/// relative to the rows that were already there.
#[derive(Debug, Clone, Copy)]
struct Baseline {
    adoptable: i64,
    finalizable: i64,
    total: i64,
}

async fn baseline(db: &Database) -> Baseline {
    Baseline {
        adoptable: db.cutover_adoptable_count().await.expect("count"),
        finalizable: db.cutover_finalizable_count().await.expect("count"),
        total: db
            .cutover_unlinked_nonterminal_count()
            .await
            .expect("count"),
    }
}

async fn seed_user(db: &Database) -> i64 {
    sqlx::query_file_scalar!(
        "tests/sql/task_execution/seed_user.sql",
        format!("handover-{}", Uuid::new_v4()),
        "Handover Test",
        "unused"
    )
    .fetch_one(db.pool())
    .await
    .expect("seed user")
}

/// One legacy task: a task the library does not execute, with one unlinked
/// item.
struct Legacy {
    user_id: i64,
    task_id: Uuid,
    item_id: Uuid,
}

async fn seed_legacy_task(db: &Database, user_id: i64) -> Legacy {
    let task_id = Uuid::new_v4();
    // The task counts its item, so the parent projection of the seeded row is
    // the one a real submission leaves behind.
    sqlx::query(
        "INSERT INTO context69.tasks \
             (id, user_id, kind, status, total_count, queued_count) \
         VALUES ($1, $2, 'text_batch', 'queued', 1, 1)",
    )
    .bind(task_id)
    .bind(user_id)
    .execute(db.pool())
    .await
    .expect("seed the task row");
    let item_id = Uuid::new_v4();
    sqlx::query_file!(
        "tests/sql/task_execution/seed_task_item.sql",
        item_id,
        task_id
    )
    .execute(db.pool())
    .await
    .expect("seed the task item");
    Legacy {
        user_id,
        task_id,
        item_id,
    }
}

async fn set_item_state(db: &Database, item_id: Uuid, status: &str) {
    sqlx::query_file!(
        "tests/sql/task_execution/set_item_wait_state.sql",
        item_id,
        status,
        Option::<&str>::None
    )
    .execute(db.pool())
    .await
    .expect("set the item state");
}

async fn park_item_on_docling(db: &Database, item_id: Uuid) {
    sqlx::query_file!(
        "tests/sql/task_execution/set_item_wait_state.sql",
        item_id,
        "waiting",
        Some("docling")
    )
    .execute(db.pool())
    .await
    .expect("park the item on the docling dependency");
}

async fn expire_item_lease(db: &Database, item_id: Uuid) {
    sqlx::query_file!("tests/sql/task_execution/expire_item_lease.sql", item_id)
        .execute(db.pool())
        .await
        .expect("expire the domain mirror");
}

async fn item_state(db: &Database, item_id: Uuid) -> (String, Option<Uuid>, i64) {
    let row = sqlx::query_file!(
        "tests/sql/task_execution/read_item_execution_state.sql",
        item_id
    )
    .fetch_one(db.pool())
    .await
    .expect("read the item");
    (row.status, row.execution_task_id, row.execution_generation)
}

async fn segment_count(db: &Database, item_id: Uuid) -> i64 {
    sqlx::query_file_scalar!(
        "tests/sql/task_execution/count_item_segments.sql",
        item_id.to_string()
    )
    .fetch_one(db.pool())
    .await
    .expect("count the segments")
}

async fn claim(db: &Database, item_id: Uuid) -> Option<ClaimedItem> {
    let claimed = db.claim_items_fast(50).await.expect("claim the items");
    claimed.into_iter().find(|item| item.id == item_id)
}

async fn cleanup(db: &Database, fixtures: &[Legacy]) {
    for fixture in fixtures {
        sqlx::query_file!(
            "tests/sql/task_execution/delete_item_library_attempts.sql",
            fixture.item_id.to_string()
        )
        .execute(db.pool())
        .await
        .expect("delete the library attempts");
        sqlx::query_file!(
            "tests/sql/task_execution/delete_library_tasks_for_item.sql",
            fixture.item_id.to_string()
        )
        .execute(db.pool())
        .await
        .expect("delete the library task");
        sqlx::query_file!(
            "tests/sql/task_execution/delete_task_items_by_task.sql",
            fixture.task_id
        )
        .execute(db.pool())
        .await
        .expect("delete the task items");
        sqlx::query_file!(
            "tests/sql/task_execution/delete_task_by_id.sql",
            fixture.task_id
        )
        .execute(db.pool())
        .await
        .expect("delete the task");
    }
    if let Some(user_id) = fixtures.first().map(|fixture| fixture.user_id) {
        sqlx::query_file!("tests/sql/task_execution/delete_user.sql", user_id)
            .execute(db.pool())
            .await
            .expect("delete the user");
    }
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_database_without_legacy_items_starts_unchanged() {
    let db = connect().await;
    let _guard = TEST_LOCK.lock().await;
    let before = baseline(&db).await;

    let outcome = run_legacy_handover(&db, production_limits()).await;
    assert_eq!(outcome.remaining, 0);
    assert_eq!(outcome.blocker, None);
    assert!(outcome.is_complete());
    if before.total == 0 {
        assert_eq!(outcome.adopted, 0);
        assert_eq!(outcome.finalized, 0);
        assert!(
            db.cutover_readiness().await.expect("readiness").is_ready(),
            "a database without legacy items starts the engine"
        );
    }
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn the_handover_adopts_every_queued_and_waiting_legacy_item_with_one_segment() {
    let db = connect().await;
    let _guard = TEST_LOCK.lock().await;
    let before = baseline(&db).await;
    let user_id = seed_user(&db).await;
    let mut fixtures = Vec::new();
    for _ in 0..MAX_BATCH_SIZE {
        fixtures.push(seed_legacy_task(&db, user_id).await);
    }
    let waiting = seed_legacy_task(&db, user_id).await;
    set_item_state(&db, waiting.item_id, "waiting").await;
    let docling = seed_legacy_task(&db, user_id).await;
    park_item_on_docling(&db, docling.item_id).await;
    let docling_item = docling.item_id;
    fixtures.push(waiting);
    fixtures.push(docling);

    let outcome = run_legacy_handover(&db, production_limits()).await;
    assert_eq!(
        outcome.adopted,
        fixtures.len() as u64 + before.adoptable as u64,
        "every queued and waiting legacy item takes a segment"
    );
    assert_eq!(outcome.remaining, 0);
    assert_eq!(outcome.blocker, None);
    assert!(outcome.is_complete());

    for fixture in &fixtures {
        let (status, linked, generation) = item_state(&db, fixture.item_id).await;
        assert!(linked.is_some(), "the item carries a segment");
        assert_eq!(generation, 1);
        assert_eq!(
            segment_count(&db, fixture.item_id).await,
            1,
            "an adopted item never carries a second segment"
        );
        assert!(
            matches!(status.as_str(), "queued" | "waiting"),
            "the domain status is preserved, got {status}"
        );
    }

    // The Docling wait survives the handover, and it is still not claimable.
    let wait = sqlx::query_file!(
        "tests/sql/task_execution/read_item_wait_state.sql",
        docling_item
    )
    .fetch_one(db.pool())
    .await
    .expect("read the wait state");
    assert_eq!(wait.waiting_reason.as_deref(), Some("docling"));
    assert_eq!(wait.attempt_count, 2);
    assert!(wait.dependency_key.is_some());

    assert!(
        db.cutover_readiness().await.expect("readiness").is_ready(),
        "the handed-over database starts the engine"
    );

    // The claims begin: the adopted items are executable.
    let claimed = db.claim_items_fast(50).await.expect("claim the items");
    assert!(
        claimed
            .iter()
            .any(|item| fixtures.iter().any(|fixture| fixture.item_id == item.id)),
        "an adopted legacy item is claimable"
    );
    assert!(
        claimed.iter().all(|item| item.id != docling_item),
        "a Docling wait is not claimable, with or without a fresh segment"
    );

    cleanup(&db, &fixtures).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn an_expired_running_legacy_item_is_closed_and_can_be_retried_and_claimed() {
    let db = connect().await;
    let _guard = TEST_LOCK.lock().await;
    let before = baseline(&db).await;
    let user_id = seed_user(&db).await;
    let fixture = seed_legacy_task(&db, user_id).await;
    set_item_state(&db, fixture.item_id, "running").await;
    expire_item_lease(&db, fixture.item_id).await;
    let attempt_id: i64 = sqlx::query_file_scalar!(
        "tests/sql/task_execution/seed_task_attempt.sql",
        fixture.task_id,
        fixture.item_id
    )
    .fetch_one(db.pool())
    .await
    .expect("seed the open domain attempt");

    let outcome = run_legacy_handover(&db, production_limits()).await;
    assert_eq!(outcome.finalized, 1 + before.finalizable as u64);
    assert_eq!(outcome.remaining, 0);
    assert_eq!(outcome.blocker, None);

    let item = sqlx::query_file!(
        "tests/sql/task_execution/read_finalized_item.sql",
        fixture.item_id
    )
    .fetch_one(db.pool())
    .await
    .expect("read the finalized item");
    assert_eq!(item.status, "failed");
    assert_eq!(item.failure_stage.as_deref(), Some("lease"));
    assert_eq!(
        item.error_message.as_deref(),
        Some("interrupted at task execution cutover")
    );
    let attempt = sqlx::query_file!(
        "tests/sql/task_execution/read_closed_item_attempt.sql",
        attempt_id
    )
    .fetch_one(db.pool())
    .await
    .expect("read the closed attempt");
    assert_eq!(attempt.status, "interrupted");
    assert!(attempt.finished_at.is_some());
    assert_eq!(
        segment_count(&db, fixture.item_id).await,
        0,
        "the interrupted item is closed, never replayed"
    );

    // The user retries the interrupted item: the retry opens the segment in
    // the same transaction as the requeue, so the item is claimable again.
    let retried = db
        .retry_task_items(fixture.task_id, user_id)
        .await
        .expect("retry the interrupted item");
    assert_eq!(retried, vec![fixture.item_id]);
    let (status, linked, generation) = item_state(&db, fixture.item_id).await;
    assert_eq!(status, "queued");
    assert!(linked.is_some(), "the retry opens an execution segment");
    assert_eq!(generation, 1);
    assert_eq!(segment_count(&db, fixture.item_id).await, 1);

    let claimed = claim(&db, fixture.item_id)
        .await
        .expect("the retried item is claimed");
    assert!(claimed.execution_context().is_some());

    cleanup(&db, &[fixture]).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn the_handover_stops_at_its_batch_bound_and_reports_the_remainder() {
    let db = connect().await;
    let _guard = TEST_LOCK.lock().await;
    let before = baseline(&db).await;
    let user_id = seed_user(&db).await;
    let mut fixtures = Vec::new();
    for _ in 0..=MAX_BATCHES {
        fixtures.push(seed_legacy_task(&db, user_id).await);
    }

    let bounded = run_legacy_handover(
        &db,
        HandoverLimits {
            batch_size: 1,
            max_batches: MAX_BATCHES,
            max_items: MAX_BATCHES,
        },
    )
    .await;
    assert_eq!(bounded.adopted, MAX_BATCHES as u64);
    assert_eq!(bounded.remaining, 1 + before.total);
    assert_eq!(bounded.blocker, Some(HandoverBlocker::Bounded));
    assert!(!bounded.is_complete());
    assert_eq!(
        db.cutover_readiness()
            .await
            .expect("readiness")
            .blocker_code(),
        Some("legacy-items"),
        "a bounded handover leaves the engine closed"
    );

    // The next start resumes the remainder without duplicating a segment.
    let resumed = run_legacy_handover(&db, production_limits()).await;
    assert_eq!(resumed.adopted, 1 + before.adoptable as u64);
    assert_eq!(resumed.remaining, 0);
    assert_eq!(resumed.blocker, None);
    for fixture in &fixtures {
        assert_eq!(
            segment_count(&db, fixture.item_id).await,
            1,
            "no item ever carries a second segment"
        );
    }

    cleanup(&db, &fixtures).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_live_execution_lease_refuses_the_automatic_handover() {
    let db = connect().await;
    let _guard = TEST_LOCK.lock().await;
    let before = baseline(&db).await;
    let user_id = seed_user(&db).await;
    let legacy = seed_legacy_task(&db, user_id).await;
    let live = seed_legacy_task(&db, user_id).await;
    // The live item owns a real lease: it is claimed through the library.
    db.produce_item_segment(live.item_id)
        .await
        .expect("produce the segment of the live item");
    let claimed = claim(&db, live.item_id)
        .await
        .expect("the live item is claimed");
    assert!(claimed.execution_context().is_some());

    let outcome = run_legacy_handover(&db, production_limits()).await;
    assert_eq!(outcome.blocker, Some(HandoverBlocker::LiveLease));
    assert_eq!(outcome.adopted, 0);
    assert_eq!(outcome.finalized, 0);
    assert_eq!(outcome.remaining, 1 + before.total);
    assert_eq!(
        segment_count(&db, legacy.item_id).await,
        0,
        "no item is adopted while a live execution lease is held"
    );
    assert_eq!(
        db.cutover_readiness()
            .await
            .expect("readiness")
            .blocker_code(),
        Some("legacy-items"),
        "a refused handover leaves the engine closed"
    );

    cleanup(&db, &[legacy, live]).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_retired_parent_keeps_the_legacy_item_blocked() {
    let db = connect().await;
    let _guard = TEST_LOCK.lock().await;
    let before = baseline(&db).await;
    let user_id = seed_user(&db).await;
    let fixture = seed_legacy_task(&db, user_id).await;
    sqlx::query("UPDATE context69.tasks SET deleted_at = now() WHERE id = $1")
        .bind(fixture.task_id)
        .execute(db.pool())
        .await
        .expect("retire the parent of the legacy item");

    let outcome = run_legacy_handover(&db, production_limits()).await;
    assert_eq!(outcome.adopted, before.adoptable as u64);
    assert_eq!(outcome.remaining, 1 + before.total);
    assert_eq!(outcome.blocker, Some(HandoverBlocker::Unresolved));
    assert_eq!(
        segment_count(&db, fixture.item_id).await,
        0,
        "an item of a retired parent is never adopted"
    );
    assert_eq!(
        db.cutover_readiness()
            .await
            .expect("readiness")
            .blocker_code(),
        Some("legacy-items"),
        "the unresolved row keeps the engine closed"
    );

    cleanup(&db, &[fixture]).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn two_concurrent_handovers_never_hand_the_same_item_over_twice() {
    let db = connect().await;
    let _guard = TEST_LOCK.lock().await;
    let before = baseline(&db).await;
    let user_id = seed_user(&db).await;
    let mut fixtures = Vec::new();
    for _ in 0..20 {
        fixtures.push(seed_legacy_task(&db, user_id).await);
    }

    let (first, second) = tokio::join!(
        run_legacy_handover(&db, production_limits()),
        run_legacy_handover(&db, production_limits()),
    );
    assert_eq!(
        first.adopted + second.adopted,
        fixtures.len() as u64 + before.adoptable as u64,
        "the two handovers share the work instead of doubling it"
    );
    assert_eq!(first.remaining, 0);
    assert_eq!(second.remaining, 0);
    for fixture in &fixtures {
        assert_eq!(
            segment_count(&db, fixture.item_id).await,
            1,
            "a concurrent handover never produces a second segment"
        );
    }

    cleanup(&db, &fixtures).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_repeated_handover_is_a_no_op() {
    let db = connect().await;
    let _guard = TEST_LOCK.lock().await;
    let before = baseline(&db).await;
    let user_id = seed_user(&db).await;
    let mut fixtures = Vec::new();
    for _ in 0..5 {
        fixtures.push(seed_legacy_task(&db, user_id).await);
    }

    let first = run_legacy_handover(&db, production_limits()).await;
    assert_eq!(first.adopted, 5 + before.adoptable as u64);
    assert_eq!(first.remaining, 0);
    assert_eq!(first.blocker, None);

    let repeated = run_legacy_handover(&db, production_limits()).await;
    assert_eq!(repeated.adopted, 0);
    assert_eq!(repeated.remaining, 0);
    assert_eq!(repeated.blocker, None);
    for fixture in &fixtures {
        assert_eq!(
            segment_count(&db, fixture.item_id).await,
            1,
            "a repeated handover never produces a second segment"
        );
    }

    cleanup(&db, &fixtures).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_retry_of_a_legacy_item_opens_its_segment() {
    let db = connect().await;
    let _guard = TEST_LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let fixture = seed_legacy_task(&db, user_id).await;
    set_item_state(&db, fixture.item_id, "failed").await;

    let retried = db
        .retry_task_items(fixture.task_id, user_id)
        .await
        .expect("retry the failed legacy item");
    assert_eq!(retried, vec![fixture.item_id]);
    let (status, linked, generation) = item_state(&db, fixture.item_id).await;
    assert_eq!(status, "queued");
    assert!(linked.is_some(), "the retry opens an execution segment");
    assert_eq!(generation, 1);
    assert_eq!(segment_count(&db, fixture.item_id).await, 1);

    let claimed = claim(&db, fixture.item_id)
        .await
        .expect("the retried legacy item is claimed");
    assert!(claimed.execution_context().is_some());

    // A retry that requeues nothing opens no segment at all.
    let empty = db
        .retry_task_items(fixture.task_id, user_id)
        .await
        .expect("retry a task without a failed item");
    assert!(empty.is_empty());
    assert_eq!(segment_count(&db, fixture.item_id).await, 1);

    cleanup(&db, &[fixture]).await;
}

/// The startup hook hands the legacy items over before the engine starts.
///
/// This boots the real application, so it belongs to the opt-in application
/// startup window: the endpoints are unreachable loopback sentinels and the
/// file library uses an isolated temporary root. The seeded item waits on a
/// Docling dependency, so the booted engine never executes it and the case
/// observes the handover itself.
#[tokio::test(flavor = "current_thread")]
#[ignore = "requires configured PostgreSQL and CONTEXT69_TEST_APP_STARTUP"]
async fn the_app_startup_hands_the_legacy_items_over() {
    let db = connect().await;
    let _guard = TEST_LOCK.lock().await;
    let user_id = seed_user(&db).await;
    let fixture = seed_legacy_task(&db, user_id).await;
    park_item_on_docling(&db, fixture.item_id).await;
    assert!(
        db.cutover_readiness()
            .await
            .expect("readiness before the boot")
            .blocker_code()
            == Some("legacy-items"),
        "the legacy item blocks the engine before the boot"
    );

    let marker = Uuid::new_v4().simple().to_string();
    let storage_root = std::env::temp_dir().join(format!("context69-handover-startup-{marker}"));
    let mut config = Config::default();
    config.app_db.url = context69_test_support::accessor::database_url();
    config.qdrant.url = SENTINEL_URL.to_string();
    config.qdrant.collection_name = format!("handover_startup_{marker}");
    config.embedding.base_url = SENTINEL_EMBEDDING_URL.to_string();
    config.embedding.model = format!("handover-startup-{marker}");
    config.embedding.dimensions = 128;
    config.file_library.storage_root = storage_root.clone();
    config.scheduler.job_id = format!("handover-startup-{marker}");
    config.auth.bootstrap_admin = None;

    let app = Context69App::with_database(config, db.clone())
        .await
        .expect("the application starts on a database with legacy items");
    assert!(
        app.tasks.execution_readiness().await.is_ready(),
        "the handed-over database starts the engine"
    );
    drop(app);

    let (status, linked, generation) = item_state(&db, fixture.item_id).await;
    assert_eq!(status, "waiting", "the domain status is preserved");
    assert!(
        linked.is_some(),
        "the startup hook linked the legacy item before the engine started"
    );
    assert_eq!(generation, 1);
    assert_eq!(segment_count(&db, fixture.item_id).await, 1);

    cleanup(&db, &[fixture]).await;
    let _ = std::fs::remove_dir_all(&storage_root);
}
