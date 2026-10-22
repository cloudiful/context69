//! The latched startup gate and the bounded legacy handover (issue 1020 P4A).
//!
//! The task engine replaced a pre-cutover execution authority that owned items
//! without a library segment. Two authorities must never run the same item, so
//! the engine decides once, before its first effect, whether any item is still
//! unlinked: while one is, no dispatcher, no maintenance and no Docling
//! recovery starts, and the operator hands the rows over with the explicit
//! tool. These cases pin that contract on the real rows: the gate blocks, the
//! blocked loops write nothing, the adoption links a fresh segment while
//! preserving the domain state, a live lease still refuses every write, and the
//! finalize closes an abandoned running item atomically without replaying it.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use context69::db::{CreateTaskSubmissionRequest, Database};
use context69::services::tasks::cutover::{
    CutoverCli, CutoverCliErrorKind, CutoverCommand, MAX_HANDOVER_ITEMS, MAX_INVENTORY_PAGE,
    parse_cutover_args, run_cutover,
};
use context69::{
    chunking::ChunkingConfig,
    config::FileLibraryConfig,
    services::{
        document_store::DocumentStoreService,
        library::{LibraryService, LibraryServiceConfig},
        namespace::NamespaceService,
        secret_store,
        settings::SettingsService,
        source_folders::SourceFoldersService,
        sync::SyncService,
        tasks::{TaskService, TaskServiceDependencies},
        translation::TranslationPublisherAdapter,
    },
};
use context69_translation::{TranslationDependencies, TranslationReadiness, TranslationService};
use serde_json::json;
use uuid::Uuid;

static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Readiness only: the cutover contract never runs translation.
struct NotReady;

#[async_trait]
impl TranslationReadiness for NotReady {
    async fn is_ready(&self) -> Result<bool> {
        Ok(false)
    }
}

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

/// The production service graph with no network runtimes, so the startup latch
/// gates the real loops.
async fn build_task_service(db: &Database) -> TaskService {
    let chunking = ChunkingConfig {
        max_chars: 1000,
        overlap_chars: 100,
    };
    let translation = TranslationService::new(TranslationDependencies {
        pool: db.pool().clone(),
        http_client: reqwest::Client::new(),
        publisher: Arc::new(TranslationPublisherAdapter::new(
            None,
            None,
            chunking.clone(),
        )),
        concurrency: 1,
        readiness: Arc::new(NotReady),
    });
    let storage_root = std::env::temp_dir().join(format!("context69-cutover-{}", Uuid::new_v4()));
    let library = LibraryService::new(
        db.clone(),
        None,
        None,
        LibraryServiceConfig {
            chunking: chunking.clone(),
            file_library: FileLibraryConfig {
                storage_root,
                max_upload_size_mb: 1,
                max_upload_request_size_mb: 1,
                ingest_concurrency: 1,
                url_import_concurrency: 1,
                url_import_min_interval_ms: 1000,
                trusted_proxy_enabled: false,
                s3: None,
            },
            valkey_url: None,
            embedding_vector_configured: false,
            embedding_vector_configuration_fingerprint: "cutover-test".to_string(),
        },
        SettingsService::new(db.clone()),
        translation.clone(),
    )
    .await
    .expect("build library service");
    let sync = SyncService::new(
        db.clone(),
        secret_store::build_unkeyed(db),
        None,
        None,
        chunking,
        1,
        translation.clone(),
    );
    TaskService::new(TaskServiceDependencies {
        db: db.clone(),
        namespace: NamespaceService::new(db.clone()),
        document_store: DocumentStoreService::new(db.clone(), None, library.clone()),
        library: library.clone(),
        sync: sync.clone(),
        source_folders: SourceFoldersService::new(db.clone(), library, sync),
        translation,
        concurrency: 1,
    })
}

/// One seeded task with a single linked item.
struct Fixture {
    user_id: i64,
    group_id: i64,
    task_id: Uuid,
    item_id: Uuid,
    file_id: Option<Uuid>,
}

async fn seed_user(db: &Database) -> i64 {
    sqlx::query_file_scalar!(
        "tests/sql/task_execution/seed_user.sql",
        format!("cutover-{}", Uuid::new_v4()),
        "Cutover Test",
        "unused"
    )
    .fetch_one(db.pool())
    .await
    .expect("seed user")
}

async fn seed_group(db: &Database) -> i64 {
    sqlx::query_file_scalar!(
        "tests/sql/task_execution/seed_group.sql",
        format!("cutover-{}", Uuid::new_v4()),
        "Cutover Test Group",
        format!("test/cutover-{}", Uuid::new_v4())
    )
    .fetch_one(db.pool())
    .await
    .expect("seed group")
}

/// A task whose item carries a library file, so the failure projection of the
/// handover has a business row to update.
async fn seed_task(db: &Database, with_file: bool) -> Fixture {
    let user_id = seed_user(db).await;
    let group_id = seed_group(db).await;
    sqlx::query_file!(
        "tests/sql/task_execution/seed_group_membership.sql",
        group_id,
        user_id
    )
    .execute(db.pool())
    .await
    .expect("seed group membership");
    let task_id = Uuid::new_v4();
    let (_, _, items) = db
        .create_task_submission_with_input_objects(CreateTaskSubmissionRequest {
            task_id,
            user_id,
            group_id: Some(group_id),
            kind: "text_batch",
            group_path: Some("test/cutover"),
            source_key: None,
            payloads: &[json!({"external_id": "a"})],
            input_storage_object_ids: None,
            idempotency_key: None,
            request_hash: &format!("cutover-{}", Uuid::new_v4()),
        })
        .await
        .expect("create task");
    let item_id = items[0];
    let file_id = if with_file {
        let file_id = Uuid::new_v4();
        sqlx::query_file!(
            "tests/sql/task_execution/seed_file_with_ingest_status.sql",
            file_id,
            group_id,
            format!("file-{file_id}.txt")
        )
        .execute(db.pool())
        .await
        .expect("seed file");
        sqlx::query_file!(
            "tests/sql/task_execution/attach_file_to_item.sql",
            item_id,
            file_id
        )
        .execute(db.pool())
        .await
        .expect("attach the file to the item");
        Some(file_id)
    } else {
        None
    };
    Fixture {
        user_id,
        group_id,
        task_id,
        item_id,
        file_id,
    }
}

/// Removes the execution segment of the item, so it has the shape the
/// pre-cutover engine left behind: an item the library does not execute, which
/// the migration marks with the zero generation.
async fn detach_segment(db: &Database, item_id: Uuid) {
    sqlx::query_file!("tests/sql/task_execution/detach_item_segment.sql", item_id)
        .execute(db.pool())
        .await
        .expect("detach the segment");
    sqlx::query_file!(
        "tests/sql/task_execution/delete_item_library_attempts.sql",
        item_id.to_string()
    )
    .execute(db.pool())
    .await
    .expect("drop the library attempts of the item");
    sqlx::query_file!(
        "tests/sql/task_execution/delete_library_tasks_for_item.sql",
        item_id.to_string()
    )
    .execute(db.pool())
    .await
    .expect("drop the library task of the item");
}

async fn set_item(db: &Database, item_id: Uuid, status: &str, waiting_reason: Option<&str>) {
    sqlx::query_file!(
        "tests/sql/task_execution/set_item_wait_state.sql",
        item_id,
        status,
        waiting_reason
    )
    .execute(db.pool())
    .await
    .expect("set the item state");
}

async fn expire_leases(db: &Database, item_id: Uuid) {
    sqlx::query_file!("tests/sql/task_execution/expire_item_lease.sql", item_id)
        .execute(db.pool())
        .await
        .expect("expire the domain mirror");
    sqlx::query_file!(
        "tests/sql/task_execution/expire_item_library_attempt.sql",
        item_id
    )
    .execute(db.pool())
    .await
    .expect("expire the library attempt");
}

async fn item_row(db: &Database, item_id: Uuid) -> (String, Option<Uuid>, i64) {
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

async fn claim(db: &Database, item_id: Uuid) -> context69::db::ClaimedItem {
    db.claim_items_fast(50)
        .await
        .expect("claim the item")
        .into_iter()
        .find(|item| item.id == item_id)
        .expect("the item is claimed")
}

async fn cleanup(db: &Database, fixture: &Fixture) {
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
    sqlx::query_file!(
        "tests/sql/task_execution/delete_group_files.sql",
        fixture.group_id
    )
    .execute(db.pool())
    .await
    .expect("delete the files");
    sqlx::query_file!(
        "tests/sql/task_execution/delete_library_tasks_for_item.sql",
        fixture.item_id.to_string()
    )
    .execute(db.pool())
    .await
    .expect("delete the library task");
    sqlx::query_file!(
        "tests/sql/task_execution/delete_group.sql",
        fixture.group_id
    )
    .execute(db.pool())
    .await
    .expect("delete the group");
    sqlx::query_file!("tests/sql/task_execution/delete_user.sql", fixture.user_id)
        .execute(db.pool())
        .await
        .expect("delete the user");
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn legacy_items_block_the_readiness_decision_until_they_are_handed_over() {
    let db = connect().await;
    let _guard = TEST_LOCK.lock().await;
    let baseline = db
        .cutover_unlinked_nonterminal_count()
        .await
        .expect("count the legacy items");

    let fixture = seed_task(&db, false).await;
    detach_segment(&db, fixture.item_id).await;
    set_item(&db, fixture.item_id, "queued", None).await;

    let blocked = db.cutover_readiness().await.expect("readiness");
    assert!(
        !blocked.is_ready(),
        "an unlinked queued item must block the engine"
    );
    assert_eq!(blocked.blocker_code(), Some("legacy-items"));
    assert_eq!(blocked.unlinked_item_count(), baseline + 1);

    let adopted = db
        .cutover_adopt(&[fixture.item_id])
        .await
        .expect("adopt the legacy item");
    assert_eq!(adopted.adopted, 1);
    assert_eq!(adopted.already_linked, 0);
    assert_eq!(
        db.cutover_unlinked_nonterminal_count()
            .await
            .expect("count after adoption"),
        baseline
    );
    // The per-target reset leaves the owned schema empty, so the baseline is
    // zero here; a database that still carries unrelated legacy rows keeps
    // reporting them, because the decision follows the count.
    let after = db
        .cutover_readiness()
        .await
        .expect("readiness after adoption");
    if baseline == 0 {
        assert!(after.is_ready(), "a handed-over database may start");
    } else {
        assert_eq!(after.blocker_code(), Some("legacy-items"));
    }

    cleanup(&db, &fixture).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_database_without_the_library_schema_blocks_as_a_library_problem() {
    let db = connect().await;
    let _guard = TEST_LOCK.lock().await;
    let current = db
        .current_database_name()
        .await
        .expect("the current database name");
    let candidates: Vec<String> = sqlx::query_file_scalar!(
        "tests/sql/task_execution/list_sibling_databases.sql",
        &current
    )
    .fetch_all(db.pool())
    .await
    .expect("list the sibling databases");

    let mut observed = 0;
    for name in candidates {
        let Some(url) = sibling_url(&context69_test_support::accessor::database_url(), &name)
        else {
            continue;
        };
        let Ok(sibling) = Database::connect_operator(&url, true).await else {
            continue;
        };
        if sibling.cutover_library_ready().await {
            // The sibling already carries the library schema; it is not the case.
            continue;
        }
        observed += 1;
        let decision = sibling.cutover_readiness().await.expect("readiness");
        assert_eq!(
            decision.blocker_code(),
            Some("library"),
            "a database without the library schema must block as a library problem"
        );
        break;
    }
    assert!(
        observed > 0,
        "no reachable database without the library schema was found; the library branch of the \
         readiness decision could not be observed end to end"
    );
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_latched_service_stays_blocked_after_the_handover_until_it_restarts() {
    let db = connect().await;
    let _guard = TEST_LOCK.lock().await;
    let fixture = seed_task(&db, false).await;
    detach_segment(&db, fixture.item_id).await;
    set_item(&db, fixture.item_id, "queued", None).await;

    let service = build_task_service(&db).await;
    assert_eq!(
        service.execution_readiness().await.blocker_code(),
        Some("legacy-items")
    );

    db.cutover_adopt(&[fixture.item_id])
        .await
        .expect("adopt the legacy item");

    // The decision was taken once: the handover under the running process does
    // not open the engine, and remediation always ends with a restart.
    assert_eq!(
        service.execution_readiness().await.blocker_code(),
        Some("legacy-items")
    );
    assert_eq!(
        service
            .execution_readiness_decided()
            .map(|d| d.blocker_code()),
        Some(Some("legacy-items"))
    );

    let restarted = build_task_service(&db).await;
    let decision = restarted.execution_readiness().await;
    assert!(
        decision.is_ready(),
        "a restarted service takes a fresh decision and finds no legacy item left, got {decision:?}"
    );

    cleanup(&db, &fixture).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_blocked_service_starts_no_dispatcher_and_no_maintenance_effect() {
    let db = connect().await;
    let _guard = TEST_LOCK.lock().await;
    let fixture = seed_task(&db, false).await;
    let item = claim(&db, fixture.item_id).await;
    assert!(item.execution_context().is_some());
    detach_segment(&db, fixture.item_id).await;
    set_item(&db, fixture.item_id, "running", None).await;
    expire_leases(&db, fixture.item_id).await;

    let service = build_task_service(&db).await;
    let shutdown = tokio_util::sync::CancellationToken::new();
    service.resume_pending();
    service.start_maintenance_with_shutdown(shutdown.clone());
    tokio::time::sleep(std::time::Duration::from_millis(750)).await;

    // A live engine would have requeued the expired item in its startup
    // maintenance and produced a claimable segment for it. The latch refused
    // both before either ran.
    let (status, segment, generation) = item_row(&db, fixture.item_id).await;
    assert_eq!(status, "running", "no maintenance effect touched the item");
    assert_eq!(segment, None, "no claim linked the item");
    assert_eq!(generation, 0, "no claim advanced the generation");
    assert_eq!(
        segment_count(&db, fixture.item_id).await,
        0,
        "the engine produced no execution segment for the legacy item"
    );

    shutdown.cancel();
    cleanup(&db, &fixture).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn inventory_is_read_only_and_bounded() {
    let db = connect().await;
    let _guard = TEST_LOCK.lock().await;
    let fixture = seed_task(&db, false).await;
    detach_segment(&db, fixture.item_id).await;
    set_item(&db, fixture.item_id, "queued", None).await;
    let before = item_row(&db, fixture.item_id).await;

    let rows = db
        .cutover_inventory(MAX_INVENTORY_PAGE, 0)
        .await
        .expect("inventory");
    let row = rows
        .iter()
        .find(|row| row.item_id == fixture.item_id)
        .expect("the legacy item is listed");
    assert_eq!(row.status, "queued");
    assert_eq!(row.kind, "text_batch");
    assert_eq!(row.ordinal, 0);
    assert!(!row.parent_retired);
    assert!(row.execution_task_id.is_none());
    assert!(!row.docling_waiting);

    assert!(
        db.cutover_inventory(MAX_INVENTORY_PAGE + 1, 0)
            .await
            .is_err()
    );
    assert!(db.cutover_inventory(MAX_INVENTORY_PAGE, -1).await.is_err());
    assert_eq!(item_row(&db, fixture.item_id).await, before);

    cleanup(&db, &fixture).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn adopt_links_a_fresh_segment_and_repeats_idempotently() {
    let db = connect().await;
    let _guard = TEST_LOCK.lock().await;
    let fixture = seed_task(&db, false).await;
    detach_segment(&db, fixture.item_id).await;
    set_item(&db, fixture.item_id, "queued", None).await;
    assert_eq!(segment_count(&db, fixture.item_id).await, 0);

    let outcome = db.cutover_adopt(&[fixture.item_id]).await.expect("adopt");
    assert_eq!(outcome.adopted, 1);
    assert_eq!(segment_count(&db, fixture.item_id).await, 1);
    let (status, linked, generation) = item_row(&db, fixture.item_id).await;
    assert_eq!(status, "queued", "the domain status is preserved");
    assert!(linked.is_some());
    assert_eq!(generation, 1);

    let repeated = db
        .cutover_adopt(&[fixture.item_id])
        .await
        .expect("repeated adoption");
    assert_eq!(repeated.adopted, 0, "a linked item is never adopted twice");
    assert_eq!(repeated.already_linked, 1);
    assert_eq!(
        segment_count(&db, fixture.item_id).await,
        1,
        "a repeated adoption never produces a second segment"
    );

    cleanup(&db, &fixture).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn adopt_preserves_a_docling_wait_without_making_it_claimable() {
    let db = connect().await;
    let _guard = TEST_LOCK.lock().await;
    let fixture = seed_task(&db, false).await;
    detach_segment(&db, fixture.item_id).await;
    set_item(&db, fixture.item_id, "waiting", Some("docling")).await;
    let row = sqlx::query_file!(
        "tests/sql/task_execution/read_item_wait_state.sql",
        fixture.item_id
    )
    .fetch_one(db.pool())
    .await
    .expect("read the wait state");
    let attempt_count: i32 = row.attempt_count;
    let dependency_key: Option<String> = row.dependency_key;
    let next_attempt_at: Option<chrono::DateTime<chrono::Utc>> = row.next_attempt_at;

    db.cutover_adopt(&[fixture.item_id])
        .await
        .expect("adopt the waiting item");

    let (status, linked, generation) = item_row(&db, fixture.item_id).await;
    let after = sqlx::query_file!(
        "tests/sql/task_execution/read_item_wait_state.sql",
        fixture.item_id
    )
    .fetch_one(db.pool())
    .await
    .expect("read the wait state after adoption");
    assert_eq!(status, "waiting", "the domain wait is preserved");
    assert_eq!(generation, 1);
    assert!(linked.is_some());
    assert_eq!(after.attempt_count, attempt_count);
    assert_eq!(after.dependency_key, dependency_key);
    assert_eq!(after.next_attempt_at, next_attempt_at);
    assert_eq!(after.waiting_reason.as_deref(), Some("docling"));

    // Adoption links a segment; it does not admit the item. The domain
    // admission still excludes the Docling wait.
    let claimed = db.claim_items_fast(50).await.expect("claim nothing");
    assert!(
        claimed.iter().all(|item| item.id != fixture.item_id),
        "a Docling wait is not claimable, with or without a fresh segment"
    );

    cleanup(&db, &fixture).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn adopt_refuses_a_running_item_and_leaves_no_segment_behind() {
    let db = connect().await;
    let _guard = TEST_LOCK.lock().await;
    let fixture = seed_task(&db, false).await;
    claim(&db, fixture.item_id).await;
    detach_segment(&db, fixture.item_id).await;
    set_item(&db, fixture.item_id, "running", None).await;

    let refused = db.cutover_adopt(&[fixture.item_id]).await;
    assert!(refused.is_err(), "a running item is never adopted");
    assert_eq!(segment_count(&db, fixture.item_id).await, 0);
    assert_eq!(item_row(&db, fixture.item_id).await.1, None);

    cleanup(&db, &fixture).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn adopt_refuses_the_whole_batch_when_one_item_is_ineligible() {
    let db = connect().await;
    let _guard = TEST_LOCK.lock().await;
    let eligible = seed_task(&db, false).await;
    detach_segment(&db, eligible.item_id).await;
    set_item(&db, eligible.item_id, "queued", None).await;
    let terminal = seed_task(&db, false).await;
    detach_segment(&db, terminal.item_id).await;
    set_item(&db, terminal.item_id, "succeeded", None).await;

    let refused = db
        .cutover_adopt(&[eligible.item_id, terminal.item_id])
        .await;
    assert!(refused.is_err(), "a terminal item refuses the batch");
    assert_eq!(
        segment_count(&db, eligible.item_id).await,
        0,
        "the eligible item of a refused batch is left untouched"
    );
    assert_eq!(item_row(&db, eligible.item_id).await.1, None);

    cleanup(&db, &eligible).await;
    cleanup(&db, &terminal).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn a_live_execution_lease_refuses_every_handover_write() {
    let db = connect().await;
    let _guard = TEST_LOCK.lock().await;
    let live = seed_task(&db, false).await;
    claim(&db, live.item_id).await;
    let legacy = seed_task(&db, false).await;
    detach_segment(&db, legacy.item_id).await;
    set_item(&db, legacy.item_id, "queued", None).await;

    let leases = db
        .cutover_live_execution_leases()
        .await
        .expect("read the leases");
    assert!(leases.any(), "a claimed item holds a live lease");

    let refused = db.cutover_adopt(&[legacy.item_id]).await;
    assert!(refused.is_err(), "a live lease refuses the handover write");
    assert_eq!(segment_count(&db, legacy.item_id).await, 0);

    // The tool refuses for the same reason even though the operator asserted
    // that every worker is stopped.
    let database = db.current_database_name().await.expect("database name");
    let cli = CutoverCli {
        command: CutoverCommand::Adopt {
            item_ids: vec![legacy.item_id],
        },
        expect_database: database,
        workers_stopped: true,
    };
    let refusal = run_cutover(&db, &cli)
        .await
        .expect_err("the tool refuses a live lease");
    assert_eq!(refusal.kind, CutoverCliErrorKind::Safety);
    assert_eq!(refusal.exit_code(), 3);

    cleanup(&db, &live).await;
    cleanup(&db, &legacy).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn finalize_interrupted_closes_an_abandoned_running_item_atomically() {
    let db = connect().await;
    let _guard = TEST_LOCK.lock().await;
    let fixture = seed_task(&db, true).await;
    claim(&db, fixture.item_id).await;
    detach_segment(&db, fixture.item_id).await;
    set_item(&db, fixture.item_id, "running", None).await;
    expire_leases(&db, fixture.item_id).await;
    let attempt_id: i64 = sqlx::query_file_scalar!(
        "tests/sql/task_execution/read_open_item_attempt.sql",
        fixture.item_id
    )
    .fetch_one(db.pool())
    .await
    .expect("the open domain attempt");

    let outcome = db
        .cutover_finalize_interrupted(&[fixture.item_id])
        .await
        .expect("finalize the interrupted item");
    assert_eq!(outcome.finalized, 1);

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
    assert!(item.lease_token.is_none());
    assert!(item.lease_until.is_none());
    assert!(item.finished_at.is_some());

    let attempt = sqlx::query_file!(
        "tests/sql/task_execution/read_closed_item_attempt.sql",
        attempt_id
    )
    .fetch_one(db.pool())
    .await
    .expect("read the closed attempt");
    assert_eq!(attempt.status, "interrupted");
    assert_eq!(attempt.failure_stage.as_deref(), Some("lease"));
    assert!(attempt.finished_at.is_some());

    // The parent was recomputed in the same transaction.
    let parent = sqlx::query_file!(
        "tests/sql/task_execution/read_parent_state.sql",
        fixture.task_id
    )
    .fetch_one(db.pool())
    .await
    .expect("read the parent");
    assert_eq!(parent.status, "failed");
    assert_eq!(parent.failed_count, 1);

    // The business row follows the existing failure projection.
    let file = sqlx::query_file!(
        "tests/sql/task_execution/read_file_status_and_error.sql",
        fixture.file_id.expect("the fixture carries a file")
    )
    .fetch_one(db.pool())
    .await
    .expect("read the file");
    assert_eq!(file.ingest_status, "failed");
    assert_eq!(
        file.error_message.as_deref(),
        Some("interrupted at task execution cutover")
    );

    assert_eq!(
        segment_count(&db, fixture.item_id).await,
        0,
        "the finalize creates no segment and replays nothing"
    );

    cleanup(&db, &fixture).await;

    // The failure follows the domain retry policy of a lease failure: an item
    // whose attempt budget is exhausted is not retryable.
    let exhausted = seed_task(&db, false).await;
    claim(&db, exhausted.item_id).await;
    detach_segment(&db, exhausted.item_id).await;
    set_item(&db, exhausted.item_id, "running", None).await;
    sqlx::query_file!(
        "tests/sql/task_execution/exhaust_item_attempt_budget.sql",
        exhausted.item_id
    )
    .execute(db.pool())
    .await
    .expect("exhaust the attempt budget");
    expire_leases(&db, exhausted.item_id).await;
    db.cutover_finalize_interrupted(&[exhausted.item_id])
        .await
        .expect("finalize the exhausted item");
    let retryable: bool = sqlx::query_file_scalar!(
        "tests/sql/task_execution/read_item_retryable.sql",
        exhausted.item_id
    )
    .fetch_one(db.pool())
    .await
    .expect("read the retry policy of the item");
    assert!(!retryable, "an exhausted item is not retryable");

    cleanup(&db, &exhausted).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn finalize_interrupted_refuses_a_linked_or_leased_item() {
    let db = connect().await;
    let _guard = TEST_LOCK.lock().await;
    let linked = seed_task(&db, false).await;
    claim(&db, linked.item_id).await;
    expire_leases(&db, linked.item_id).await;

    let refused = db.cutover_finalize_interrupted(&[linked.item_id]).await;
    assert!(
        refused.is_err(),
        "a linked running item is not an abandoned legacy row"
    );

    cleanup(&db, &linked).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn the_tool_refuses_a_target_that_is_not_the_expected_database() {
    let db = connect().await;
    let _guard = TEST_LOCK.lock().await;
    let cli = CutoverCli {
        command: CutoverCommand::Inventory {
            limit: 1,
            offset: 0,
        },
        expect_database: "definitely-not-this-database".to_string(),
        workers_stopped: false,
    };
    let refusal = run_cutover(&db, &cli)
        .await
        .expect_err("the tool refuses a foreign target");
    assert_eq!(refusal.kind, CutoverCliErrorKind::Target);
    assert_eq!(refusal.exit_code(), 2);
}

/// Rewrites the database name of a connection string without echoing it.
fn sibling_url(url: &str, database: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let (authority, after) = match rest.split_once('/') {
        Some((authority, after)) => (authority, after),
        None => (rest, ""),
    };
    let query = after
        .find('?')
        .map(|index| after[index..].to_string())
        .unwrap_or_default();
    Some(format!("{scheme}://{authority}/{database}{query}"))
}

#[test]
fn the_cutover_arguments_are_refused_before_any_connection() {
    let parse = |args: &[&str]| {
        let owned: Vec<String> = args.iter().map(|arg| (*arg).to_string()).collect();
        parse_cutover_args(&owned)
    };
    let uuid = Uuid::new_v4().to_string();

    assert!(parse(&[]).is_err());
    assert_eq!(
        parse(&["inventory"]).unwrap_err().kind,
        CutoverCliErrorKind::Usage
    );
    assert!(parse(&["inventory", "--expect-database", "context69"]).is_ok());
    let oversized_page = (MAX_INVENTORY_PAGE + 1).to_string();
    assert!(
        parse(&[
            "inventory",
            "--expect-database",
            "context69",
            "--limit",
            &oversized_page
        ])
        .is_err()
    );
    assert!(
        parse(&[
            "inventory",
            "--expect-database",
            "context69",
            "--offset",
            "-1"
        ])
        .is_err()
    );
    assert!(
        parse(&[
            "inventory",
            "--expect-database",
            "context69",
            "--workers-stopped"
        ])
        .is_err()
    );
    assert!(
        parse(&[
            "inventory",
            "--expect-database",
            "context69",
            "--item",
            &uuid
        ])
        .is_err()
    );
    assert!(parse(&["adopt", "--expect-database", "context69", "--item", &uuid]).is_err());
    assert!(parse(&["adopt", "--expect-database", "context69"]).is_err());
    assert!(parse(&["adopt", "--workers-stopped", "--item", &uuid]).is_err());
    assert!(
        parse(&[
            "adopt",
            "--expect-database",
            "context69",
            "--workers-stopped",
            "--item",
            "not-a-uuid"
        ])
        .is_err()
    );
    assert!(
        parse(&[
            "adopt",
            "--expect-database",
            "context69",
            "--workers-stopped",
            "--item",
            &uuid,
            "--item",
            &uuid
        ])
        .is_err()
    );
    let many: Vec<String> = (0..MAX_HANDOVER_ITEMS + 1)
        .flat_map(|_| vec!["--item".to_string(), Uuid::new_v4().to_string()])
        .collect();
    let mut args = vec![
        "adopt".to_string(),
        "--expect-database".to_string(),
        "context69".to_string(),
        "--workers-stopped".to_string(),
    ];
    args.extend(many);
    assert!(parse_cutover_args(&args).is_err());
    assert!(parse(&["unknown", "--expect-database", "context69"]).is_err());
    assert!(
        parse(&[
            "adopt",
            "--expect-database",
            "context69",
            "--workers-stopped",
            "--item",
            &uuid,
            "--limit",
            "1"
        ])
        .is_err()
    );

    let parsed = parse(&[
        "finalize-interrupted",
        "--expect-database",
        " context69 ",
        "--workers-stopped",
        "--item",
        &uuid,
    ])
    .expect("a valid finalize invocation");
    assert_eq!(parsed.expect_database, " context69 ");
    assert!(parsed.workers_stopped);
    assert!(matches!(
        parsed.command,
        CutoverCommand::FinalizeInterrupted { .. }
    ));
}

#[test]
fn the_configured_database_name_is_read_but_never_echoed() {
    use context69::services::tasks::cutover::{Sanitized, configured_database_name};

    assert_eq!(
        configured_database_name("postgres://user:secret@host:5432/context69?sslmode=require")
            .expect("a name"),
        "context69"
    );
    assert!(configured_database_name("postgres://user:secret@host:5432").is_err());
    assert!(configured_database_name("").is_err());

    let redacted =
        Sanitized("connection error: postgres://app:hunter2@db.internal:5432/context69 refused")
            .to_string();
    assert!(!redacted.contains("hunter2"), "{redacted}");
    assert!(
        redacted.contains("postgres://db.internal:5432/context69"),
        "{redacted}"
    );
}
