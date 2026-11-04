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

use std::panic::AssertUnwindSafe;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use cloudiful_tasks::{EnqueueOutcome, EnqueueRequest, TaskId, TaskKind};
use context69::db::{CreateTaskSubmissionRequest, Database, SegmentPayload};
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
use context69_test_support::execution_cleanup::{
    FixtureExecutionScope, quiesce_fixture_executions,
};
use context69_test_support::library_policy::finish_fixture;
use context69_translation::{TranslationDependencies, TranslationReadiness, TranslationService};
use futures::FutureExt;
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
    let claimed = db.claim_items_fast().await.expect("claim the item");
    assert_eq!(
        claimed.len(),
        1,
        "the global claim must return exactly the fixture item, not a sibling fixture"
    );
    claimed
        .into_iter()
        .find(|item| item.id == item_id)
        .expect("the item is claimed")
}

/// Deterministically makes an owned item point at a fresh queued library
/// segment with default (unclassified) metadata.
///
/// The classified segment is cancelled through the public queue API first, the
/// unclassified segment is enqueued with the normal payload and idempotency key,
/// and only then is the domain pointer relinked under its exact old fence. No
/// library row is updated directly.
async fn seed_unclassified_legacy_link(db: &Database, fixture: &Fixture) -> (Uuid, i64) {
    let (_, old_segment, old_generation) = item_row(db, fixture.item_id).await;
    let old_segment = old_segment.expect("the item starts linked");
    let queue = db
        .execution_queue()
        .await
        .expect("load the execution queue");
    let mut tx = db.pool().begin().await.expect("begin the fixture");
    queue
        .request_cancel_in(&mut tx, TaskId::from_uuid(old_segment))
        .await
        .expect("cancel the classified segment");
    let generation = old_generation + 1;
    let payload = SegmentPayload::new(fixture.item_id, fixture.task_id, generation);
    let request = EnqueueRequest::new(
        TaskKind::new("context69.item").expect("the segment kind is valid"),
        payload.to_value(),
    )
    .with_idempotency_key(SegmentPayload::idempotency_key(fixture.item_id, generation))
    .expect("the segment key is valid");
    let segment = match queue
        .enqueue_in(&mut tx, request)
        .await
        .expect("enqueue the unclassified fixture segment")
    {
        EnqueueOutcome::Created(task) | EnqueueOutcome::Existing(task) => task.id().as_uuid(),
    };
    let linked = sqlx::query_file_scalar!(
        "tests/sql/task_execution/admission_seed_legacy_link.sql",
        fixture.item_id,
        segment,
        generation,
        old_segment,
        old_generation
    )
    .fetch_optional(&mut *tx)
    .await
    .expect("relink the domain item");
    assert!(linked.is_some(), "the exact old pointer must still match");
    tx.commit().await.expect("commit the fixture");
    (segment, generation)
}

async fn install_admission_fault(db: &Database) {
    sqlx::raw_sql(include_str!("sql/task_execution/admission_fault_setup.sql"))
        .execute(db.pool())
        .await
        .expect("install the test-only admission fault guard");
}

async fn arm_admission_fault(db: &Database, item_id: Uuid, mode: &str) {
    sqlx::query_file!(
        "tests/sql/task_execution/admission_fault_seed.sql",
        item_id,
        mode
    )
    .fetch_one(db.pool())
    .await
    .expect("arm the test-only admission fault guard");
}

async fn cleanup_admission_fault(db: &Database) -> std::result::Result<(), String> {
    sqlx::raw_sql(include_str!(
        "sql/task_execution/admission_fault_cleanup.sql"
    ))
    .execute(db.pool())
    .await
    .map_err(|error| format!("remove the test-only admission fault guard: {error}"))?;
    Ok(())
}

async fn cleanup(db: &Database, fixture: &Fixture) {
    if let Err(error) = try_cleanup(db, fixture).await {
        panic!("{error}");
    }
}

/// Ends the fixture through the public library API before its domain rows are
/// deleted. Terminal library history is preserved; no library row is deleted
/// directly and no unlinked task is swept.
async fn try_cleanup(db: &Database, fixture: &Fixture) -> std::result::Result<(), String> {
    quiesce_fixture_executions(db.pool(), FixtureExecutionScope::Tasks(&[fixture.task_id]))
        .await
        .map_err(|error| format!("quiesce the fixture executions: {error}"))?;
    sqlx::query_file!(
        "tests/sql/task_execution/delete_task_items_by_task.sql",
        fixture.task_id
    )
    .execute(db.pool())
    .await
    .map_err(|error| format!("delete the task items: {error}"))?;
    sqlx::query_file!(
        "tests/sql/task_execution/delete_task_by_id.sql",
        fixture.task_id
    )
    .execute(db.pool())
    .await
    .map_err(|error| format!("delete the task: {error}"))?;
    sqlx::query_file!(
        "tests/sql/task_execution/delete_group_files.sql",
        fixture.group_id
    )
    .execute(db.pool())
    .await
    .map_err(|error| format!("delete the files: {error}"))?;
    sqlx::query_file!(
        "tests/sql/task_execution/delete_group.sql",
        fixture.group_id
    )
    .execute(db.pool())
    .await
    .map_err(|error| format!("delete the group: {error}"))?;
    sqlx::query_file!("tests/sql/task_execution/delete_user.sql", fixture.user_id)
        .execute(db.pool())
        .await
        .map_err(|error| format!("delete the user: {error}"))?;
    Ok(())
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
async fn adopt_all_selects_one_bounded_eligible_batch_and_reports_remaining() {
    let db = connect().await;
    let _guard = TEST_LOCK.lock().await;
    let baseline_total = db
        .cutover_unlinked_nonterminal_count()
        .await
        .expect("count the pre-existing legacy items");
    let baseline_adoptable = db
        .cutover_adoptable_count()
        .await
        .expect("count the pre-existing adoptable items");
    let first = seed_task(&db, false).await;
    detach_segment(&db, first.item_id).await;
    set_item(&db, first.item_id, "queued", None).await;
    let second = seed_task(&db, false).await;
    detach_segment(&db, second.item_id).await;
    set_item(&db, second.item_id, "waiting", None).await;
    let running = seed_task(&db, false).await;
    claim(&db, running.item_id).await;
    detach_segment(&db, running.item_id).await;
    set_item(&db, running.item_id, "running", None).await;
    expire_leases(&db, running.item_id).await;
    // Create the untouched linked fixture only after the claim above: a global
    // claim must never consume a sibling fixture and leave its live lease
    // behind for every later handover case in this binary.
    let linked = seed_task(&db, false).await;

    let before = db
        .cutover_unlinked_nonterminal_count()
        .await
        .expect("count before the bounded batch");
    assert_eq!(
        before,
        baseline_total + 3,
        "two eligible items and one running item block the engine before the batch"
    );
    assert!(
        !db.cutover_live_execution_leases()
            .await
            .expect("read the live execution leases")
            .any(),
        "the adopt-all fixture must not leave a live lease behind"
    );

    let outcome = db.cutover_adopt_all().await.expect("bounded adopt --all");
    assert_eq!(outcome.adopted, 2, "only the queued and waiting items");
    assert_eq!(outcome.already_linked, 0);
    assert_eq!(
        outcome.remaining_adoptable, baseline_adoptable,
        "both adoptable rows were taken and none remain beyond the baseline"
    );
    assert_eq!(
        outcome.total_blockers,
        baseline_total + 1,
        "the running item is never adopted and stays reported as a blocker"
    );
    assert_eq!(segment_count(&db, first.item_id).await, 1);
    assert_eq!(segment_count(&db, second.item_id).await, 1);
    assert_eq!(
        segment_count(&db, running.item_id).await,
        0,
        "adopt --all never touches a running item"
    );
    assert_eq!(item_row(&db, running.item_id).await.0, "running");
    assert_eq!(
        segment_count(&db, linked.item_id).await,
        1,
        "an already linked item is not selected"
    );

    let repeated = db
        .cutover_adopt_all()
        .await
        .expect("a repeated bounded adopt --all");
    assert_eq!(repeated.adopted, 0, "a linked item is never adopted twice");
    assert_eq!(repeated.already_linked, 0);
    assert_eq!(repeated.remaining_adoptable, baseline_adoptable);
    assert_eq!(repeated.total_blockers, baseline_total + 1);
    assert_eq!(segment_count(&db, first.item_id).await, 1);
    assert_eq!(segment_count(&db, second.item_id).await, 1);

    cleanup(&db, &first).await;
    cleanup(&db, &second).await;
    cleanup(&db, &linked).await;
    cleanup(&db, &running).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn adopt_all_filters_before_the_bound_so_older_ineligible_rows_cannot_starve_it() {
    let db = connect().await;
    let _guard = TEST_LOCK.lock().await;
    let baseline_total = db
        .cutover_unlinked_nonterminal_count()
        .await
        .expect("count the pre-existing legacy items");
    let baseline_adoptable = db
        .cutover_adoptable_count()
        .await
        .expect("count the pre-existing adoptable items");

    // One task whose item is linked, plus 100 older unlinked running rows. They
    // are ineligible for adoption and sort before the eligible queued item.
    let older = seed_task(&db, false).await;
    sqlx::query(
        "INSERT INTO context69.task_items \
             (id, task_id, ordinal, payload, status, created_at) \
         SELECT gen_random_uuid(), $1, ordinal, '{}'::jsonb, 'running', \
                now() - interval '1 day' \
         FROM generate_series(1, 100) AS ordinal",
    )
    .bind(older.task_id)
    .execute(db.pool())
    .await
    .expect("seed 100 older ineligible rows");

    let eligible = seed_task(&db, false).await;
    detach_segment(&db, eligible.item_id).await;
    set_item(&db, eligible.item_id, "queued", None).await;
    let retired = seed_task(&db, false).await;
    detach_segment(&db, retired.item_id).await;
    set_item(&db, retired.item_id, "queued", None).await;
    sqlx::query("UPDATE context69.tasks SET deleted_at = now() WHERE id = $1")
        .bind(retired.task_id)
        .execute(db.pool())
        .await
        .expect("retire the parent of the queued item");

    let outcome = db
        .cutover_adopt_all()
        .await
        .expect("bounded adopt --all past the ineligible page");
    assert_eq!(
        outcome.adopted, 1,
        "the eligible row after 100 older ineligible rows is still selected"
    );
    assert_eq!(
        outcome.remaining_adoptable, baseline_adoptable,
        "the retired-parent row is never adoptable work"
    );
    assert_eq!(
        outcome.total_blockers,
        baseline_total + 101,
        "the running rows and the retired-parent row remain blockers"
    );
    assert_eq!(segment_count(&db, eligible.item_id).await, 1);
    assert_eq!(
        segment_count(&db, retired.item_id).await,
        0,
        "a retired-parent row is never adopted"
    );

    let running_unlinked: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM context69.task_items \
         WHERE task_id = $1 AND status = 'running' AND execution_task_id IS NULL",
    )
    .bind(older.task_id)
    .fetch_one(db.pool())
    .await
    .expect("count the untouched running rows");
    assert_eq!(running_unlinked, 100);

    cleanup(&db, &older).await;
    cleanup(&db, &eligible).await;
    cleanup(&db, &retired).await;
}

#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn the_normal_connect_path_leaves_the_library_schema_ready() {
    let url = context69_test_support::accessor::database_url();
    let db = Database::connect(&url)
        .await
        .expect("normal connect prepares the application and library schemas");
    assert!(
        db.pending_upgrade().is_none(),
        "the normal admission path, not recovery admission, prepared the schema"
    );
    assert!(
        db.cutover_library_ready().await,
        "normal admission leaves the library schema genuine and compatible"
    );

    let repeated = Database::connect(&url)
        .await
        .expect("a second normal connect is idempotent");
    assert!(
        repeated.cutover_library_ready().await,
        "a repeated normal connect applies nothing and keeps the schema ready"
    );
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
    let claimed = db.claim_items_fast().await.expect("claim nothing");
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
    let backlog = seed_task(&db, false).await;
    seed_unclassified_legacy_link(&db, &backlog).await;

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
        expect_database: database.clone(),
        workers_stopped: true,
    };
    let refusal = run_cutover(&db, &cli)
        .await
        .expect_err("the tool refuses a live lease");
    assert_eq!(refusal.kind, CutoverCliErrorKind::Safety);
    assert_eq!(refusal.exit_code(), 3);
    let reclassify = CutoverCli {
        command: CutoverCommand::ReclassifyAdmission { limit: 10 },
        expect_database: database,
        workers_stopped: true,
    };
    let refusal = run_cutover(&db, &reclassify)
        .await
        .expect_err("reclassification also refuses a live lease");
    assert_eq!(refusal.kind, CutoverCliErrorKind::Safety);

    cleanup(&db, &live).await;
    cleanup(&db, &legacy).await;
    cleanup(&db, &backlog).await;
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
    // whose execution-failure budget is exhausted is not retryable.
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
    assert!(
        parse(&[
            "adopt",
            "--expect-database",
            "context69",
            "--workers-stopped",
            "--all",
            "--item",
            &uuid
        ])
        .is_err()
    );
    assert!(parse(&["adopt", "--expect-database", "context69", "--all"]).is_err());
    assert!(
        parse(&[
            "adopt",
            "--expect-database",
            "context69",
            "--workers-stopped",
            "--all",
            "--all"
        ])
        .is_err()
    );
    assert!(
        parse(&[
            "finalize-interrupted",
            "--expect-database",
            "context69",
            "--workers-stopped",
            "--all"
        ])
        .is_err()
    );
    assert!(parse(&["inventory", "--expect-database", "context69", "--all"]).is_err());

    let adopt_all = parse(&[
        "adopt",
        "--expect-database",
        "context69",
        "--workers-stopped",
        "--all",
    ])
    .expect("a valid bounded adopt --all invocation");
    assert_eq!(adopt_all.command, CutoverCommand::AdoptAll);

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

/// The explicit reclassification converts one unclassified queued backlog item,
/// preserves its history, and is a no-op after completion (issue 1172 P6A2).
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn reclassify_admission_converts_unclassified_backlog_and_repeats_as_a_no_op() {
    let _guard = TEST_LOCK.lock().await;
    let db = connect().await;
    let fixture = seed_task(&db, false).await;
    let (old_segment, old_generation) = seed_unclassified_legacy_link(&db, &fixture).await;
    let segments_before_reclassification = segment_count(&db, fixture.item_id).await;

    let outcome = db
        .cutover_reclassify_admission_batch(10)
        .await
        .expect("reclassify the bounded backlog");
    assert_eq!(outcome.processed, 1);
    assert_eq!(outcome.relinked, 1);
    assert_eq!(outcome.skipped_uncertain, 0);
    assert_eq!(outcome.remaining, 0);

    let (_, new_segment, new_generation) = item_row(&db, fixture.item_id).await;
    let new_segment = new_segment.expect("the item is relinked");
    assert_ne!(new_segment, old_segment);
    assert_eq!(new_generation, old_generation + 1);
    let metadata = sqlx::query_file!(
        "tests/sql/task_execution/admission_read_segment.sql",
        new_segment
    )
    .fetch_one(db.pool())
    .await
    .expect("read the new admission metadata");
    assert_eq!(metadata.class.as_deref(), Some("interactive"));
    assert_eq!(
        metadata.resources,
        vec!["ingest".to_string(), "interactive".to_string()]
    );
    assert_eq!(metadata.status, "queued");
    let old_metadata = sqlx::query_file!(
        "tests/sql/task_execution/admission_read_segment.sql",
        old_segment
    )
    .fetch_one(db.pool())
    .await
    .expect("read the old segment");
    assert_eq!(old_metadata.status, "cancelled");
    assert_eq!(old_metadata.class, None);
    let segments_after_reclassification = segment_count(&db, fixture.item_id).await;
    assert_eq!(
        segments_after_reclassification,
        segments_before_reclassification + 1,
        "reclassification retains the original and legacy history and adds one segment"
    );

    let repeated = db
        .cutover_reclassify_admission_batch(10)
        .await
        .expect("a completed reclassification is a no-op");
    assert_eq!(repeated.processed, 0);
    assert_eq!(repeated.relinked, 0);
    assert_eq!(repeated.remaining, 0);
    assert_eq!(
        segment_count(&db, fixture.item_id).await,
        segments_after_reclassification,
        "repeating the operation creates no additional segment"
    );

    cleanup(&db, &fixture).await;
}

/// An unresolved Docling submission is never cancelled or reclassified by the
/// explicit backlog command; it is reported as skipped (issue 1172 P6A2).
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn reclassify_admission_skips_a_docling_unknown_outcome_as_inert() {
    let _guard = TEST_LOCK.lock().await;
    let db = connect().await;
    let fixture = seed_task(&db, false).await;
    let (old_segment, old_generation) = seed_unclassified_legacy_link(&db, &fixture).await;
    sqlx::query_file!(
        "tests/sql/docling_remote_jobs/seed_submitting_job.sql",
        fixture.task_id,
        fixture.item_id,
        false,
        Some(old_segment),
        Some(old_generation),
        json!({ "fixture": true }),
        "http://docling.invalid/v1",
        chrono::Utc::now() + chrono::Duration::hours(1),
    )
    .fetch_one(db.pool())
    .await
    .expect("seed an unresolved submission");
    sqlx::query_file!(
        "tests/sql/task_execution/admission_mark_submission_uncertain.sql",
        fixture.item_id,
        fixture.task_id,
        "seeded unknown outcome"
    )
    .fetch_one(db.pool())
    .await
    .expect("mark the submission unknown");

    let outcome = db
        .cutover_reclassify_admission_batch(10)
        .await
        .expect("the bounded scan reports the inert row");
    assert_eq!(outcome.processed, 1);
    assert_eq!(outcome.relinked, 0);
    assert_eq!(outcome.skipped_uncertain, 1);
    let (_, linked, generation) = item_row(&db, fixture.item_id).await;
    assert_eq!(linked, Some(old_segment));
    assert_eq!(generation, old_generation);
    let metadata = sqlx::query_file!(
        "tests/sql/task_execution/admission_read_segment.sql",
        old_segment
    )
    .fetch_one(db.pool())
    .await
    .expect("read the untouched segment");
    assert_eq!(metadata.status, "queued");
    assert_eq!(metadata.class, None);

    cleanup(&db, &fixture).await;
}

/// A failed relink fence rolls the whole explicit reclassification transaction
/// back: the old segment stays queued, the original pointer/generation survive,
/// and no new segment commits (issue 1172 P6A2).
#[tokio::test]
#[ignore = "requires configured reusable PostgreSQL"]
async fn reclassify_admission_rolls_back_when_the_relink_guard_refuses() {
    let _guard = TEST_LOCK.lock().await;
    let db = connect().await;
    let mut fixture: Option<Fixture> = None;
    let outcome = AssertUnwindSafe(async {
        install_admission_fault(&db).await;
        fixture = Some(seed_task(&db, false).await);
        let seeded = fixture.as_ref().expect("the fixture was just seeded");
        let (old_segment, old_generation) = seed_unclassified_legacy_link(&db, seeded).await;
        let segments_before = segment_count(&db, seeded.item_id).await;
        arm_admission_fault(&db, seeded.item_id, "relink").await;

        let error = db
            .cutover_reclassify_admission_batch(10)
            .await
            .expect_err("the relink guard refuses the batch");
        assert!(
            error
                .to_string()
                .contains("changed during reclassification")
        );
        assert_eq!(
            segment_count(&db, seeded.item_id).await,
            segments_before,
            "the refused batch commits no fresh segment"
        );
        let (_, pointer, generation) = item_row(&db, seeded.item_id).await;
        assert_eq!(pointer, Some(old_segment));
        assert_eq!(generation, old_generation);
        let metadata = sqlx::query_file!(
            "tests/sql/task_execution/admission_read_segment.sql",
            old_segment
        )
        .fetch_one(db.pool())
        .await
        .expect("read the rolled-back segment");
        assert_eq!(metadata.status, "queued");
        assert_eq!(metadata.class, None);

        cleanup_admission_fault(&db)
            .await
            .expect("drop the test-only fault guard");
        let positive = db
            .cutover_reclassify_admission_batch(10)
            .await
            .expect("the normal relink succeeds after the guard is removed");
        assert!(
            positive.relinked >= 1,
            "the normal relink commits the batch"
        );
        let (_, fresh_segment, fresh_generation) = item_row(&db, seeded.item_id).await;
        let fresh_segment = fresh_segment.expect("the positive control links a fresh segment");
        assert_ne!(fresh_segment, old_segment);
        assert!(fresh_generation > old_generation);
        let classifier = sqlx::query_file!(
            "tests/sql/task_execution/admission_read_segment.sql",
            fresh_segment
        )
        .fetch_one(db.pool())
        .await
        .expect("read the reclassified segment");
        assert_eq!(classifier.class.as_deref(), Some("interactive"));
    })
    .catch_unwind()
    .await;

    let mut cleanup_error = cleanup_admission_fault(&db).await.err();
    if let Some(seeded) = fixture.as_ref() {
        if let Err(error) = try_cleanup(&db, seeded).await {
            cleanup_error = Some(match cleanup_error {
                Some(existing) => format!("{existing}; {error}"),
                None => error,
            });
        }
    }
    finish_fixture(outcome, cleanup_error, Ok(()));
}
